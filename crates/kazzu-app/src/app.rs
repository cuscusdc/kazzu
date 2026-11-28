//! Montagem do app: janela Slint, controller, threads de trabalho e ponte do motor de áudio.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicI32;

use kazzu_audio::{CacheConfig, Engine, EngineConfig, NullSink};
use slint::ComponentHandle;

use crate::catalog::{Providers, YtmCatalog};
use crate::config::{Config, StreamQualityPref, ThemePref};
use crate::controller::{Controller, Msg, MsgSink, NavTab};
use crate::covers::CoverFetcher;
use crate::event_bridge::{spawn_engine_bridge, spawn_stream_bridge};
use crate::remote::RemoteWorker;
use crate::slint_view::SlintView;
use crate::ui;
use crate::worker::{QueueMode, ThreadedRunner};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("não foi possível abrir a biblioteca: {0}")]
    Library(#[from] kazzu_library::Error),
    #[error("não foi possível iniciar o motor de áudio: {0}")]
    Audio(#[from] kazzu_core::Error),
    #[error("falha ao criar a janela: {0}")]
    Window(#[from] slint::PlatformError),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("diretório de dados do usuário indisponível")]
    NoDataDir,
    /// O renderer com vidro não iniciou; quem chamou deve reabrir em modo software.
    #[error("renderer indisponível")]
    RetrySoftware,
}

/// Como iniciar.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Onde ficam banco e cache (padrão: diretório de dados do usuário).
    pub data_dir: Option<PathBuf>,
    /// Arquivo de configuração (padrão: diretório de configuração do usuário).
    pub config_path: Option<PathBuf>,
    /// Pastas de música extras a indexar já na abertura.
    pub extra_folders: Vec<PathBuf>,
    /// Saída de áudio simulada (sem placa de som): para CI e capturas de tela.
    pub null_audio: bool,
    /// O renderer ativo não suporta `backdrop-blur` (software).
    pub software_renderer: bool,
    /// Sem YouTube Music (só arquivos locais): testes e `--no-youtube`.
    pub no_remote: bool,
    /// Mostra a janela já na criação para descobrir se o renderer iniciou (falha = `AppError::RetrySoftware`).
    pub probe_renderer: bool,
}

thread_local! {
    static CONTROLLER: RefCell<Option<Rc<RefCell<Controller<SlintView>>>>> = const { RefCell::new(None) };
}

fn dispatch(msg: Msg) {
    CONTROLLER.with(|slot| {
        let Some(ctrl) = slot.borrow().clone() else { return };
        match ctrl.try_borrow_mut() {
            Ok(mut c) => c.handle(msg),
            Err(_) => log::error!("mensagem descartada: controller ocupado"),
        }
    });
}

pub struct App {
    pub window: ui::AppWindow,
    pub controller: Rc<RefCell<Controller<SlintView>>>,
}

impl App {
    pub fn build(opts: &Options) -> Result<Self, AppError> {
        // 1. Janela (e, se pedido, teste do renderer) ANTES de subir motor de áudio, worker e banco: se o OpenGL não
        //    iniciar, quem chamou reabre em modo software sem ter deixado threads nem conexões para trás.
        let window = ui::AppWindow::new()?;
        if opts.probe_renderer
            && let Err(e) = window.show()
        {
            log::warn!("renderer com vidro indisponível: {e}");
            return Err(AppError::RetrySoftware);
        }
        window.global::<ui::Theme>().set_blur_supported(!opts.software_renderer);
        window.global::<ui::AppState>().set_mobile(cfg!(target_os = "android"));
        window.global::<ui::AppState>().set_licenses_text(licenses_text().into());
        window.global::<ui::AppState>().set_version_label(format!("Kazzu {}", env!("CARGO_PKG_VERSION")).into());

        // 2. Configuração. Arquivo ilegível vai para `.bak` (nunca é sobrescrito sem aviso).
        let cfg_path = opts.config_path.clone().or_else(Config::default_path);
        let mut notice: Option<String> = None;
        let cfg = match &cfg_path {
            Some(p) => match Config::load(p) {
                Ok(c) => c,
                Err(e) => {
                    log::warn!("{e}; usando a configuração padrão");
                    notice = Some(match Config::back_up_corrupt(p) {
                        Ok(bak) => format!("A configuração estava ilegível. Guardei uma cópia em {} e voltei ao padrão.", bak.display()),
                        Err(_) => "A configuração estava ilegível; voltei ao padrão.".to_owned(),
                    });
                    Config::default()
                }
            },
            None => Config::default(),
        };
        let data_dir = match &opts.data_dir {
            Some(d) => d.clone(),
            None => kazzu_library::Library::default_data_dir().map_err(|_| AppError::NoDataDir)?,
        };

        // 3. Motor de áudio. O dispositivo só é aberto na primeira reprodução.
        let mut config = EngineConfig { volume: crate::controller::sane_unit(cfg.volume, 1.0), ..EngineConfig::default() };
        // faixas remotas ouvidas até o fim ficam em disco (base do modo offline); o limite protege o espaço do usuário
        config.stream.cache = Some(CacheConfig { dir: data_dir.join("cache").join("streams"), max_bytes: STREAM_CACHE_BYTES });
        // YouTube Music: provedor padrão, ativo sem login. Falha ao abrir o cache/biblioteca dele não derruba o app.
        let remote = if opts.no_remote {
            None
        } else {
            match YtmCatalog::open(&data_dir) {
                Ok(cat) => Some(RemoteWorker::new(Providers::new().with(Arc::new(cat)), CoverFetcher::new(Some(data_dir.join("cache").join("covers"))))),
                Err(e) => {
                    log::warn!("YouTube Music indisponível: {e}");
                    None
                }
            }
        };
        let remote_on = remote.is_some();

        let quality = Arc::new(AtomicI32::new(cfg.stream_quality.index()));
        let mut engine = Engine::builder().config(config);
        if opts.null_audio {
            engine = engine.sink(NullSink::new());
        }
        if let Some(catalog) = remote.as_ref().and_then(|worker| worker.providers().default_remote()).cloned() {
            engine = crate::catalog::install_resolver(engine, catalog, quality.clone());
        }
        let engine = engine.spawn()?;
        let sink: MsgSink = Arc::new(|msg| {
            let _ = slint::invoke_from_event_loop(move || dispatch(msg));
        });
        spawn_engine_bridge(&engine, sink.clone())?;
        spawn_stream_bridge(&engine, sink.clone())?;


        let emit_sink = sink.clone();
        let runner = ThreadedRunner::spawn(data_dir.clone(), Arc::new(move |r| emit_sink(Msg::Job(r))), remote)?;

        if !opts.software_renderer {
            watch_frames(&window, sink.clone());
        }
        window.global::<ui::AppState>().set_glass_style(cfg.glass_style);
        window.global::<ui::AppState>().set_color_palette(cfg.color_palette);
        window.global::<ui::AppState>().set_artwork_background(cfg.artwork_background);
        window.global::<ui::AppState>().set_library_view(cfg.library_view);
        window.global::<ui::AppState>().set_library_density(cfg.library_density);
        let view = SlintView::new(&window);
        let mut ctl = Controller::new(view, engine, Box::new(runner), sink.clone(), cfg, cfg_path);
        ctl.set_data_dir(data_dir);
        #[cfg(unix)]
        ctl.enable_discord_from_env();
        if remote_on {
            ctl.enable_engine_remote(quality);
        }
        let controller = Rc::new(RefCell::new(ctl));
        CONTROLLER.with(|slot| *slot.borrow_mut() = Some(controller.clone()));
        wire(&window, &controller);

        controller.borrow_mut().init(Default::default());
        if let Some(text) = notice {
            controller.borrow_mut().notice(&text, true);
        }
        // preferências do sistema chegam depois (leitura fora da thread de UI)
        let sys_sink = sink;
        let _ = std::thread::Builder::new().name("kazzu-sysprefs".into()).spawn(move || sys_sink(Msg::System(crate::platform::system_prefs())));
        for folder in &opts.extra_folders {
            controller.borrow_mut().add_folder_path(folder.clone());
        }
        Ok(Self { window, controller })
    }

    /// Mostra a janela e roda o laço de eventos até fechar; depois grava a configuração e para o motor.
    pub fn run(self) -> Result<(), AppError> {
        let result = self.window.run();
        self.finish();
        result.map_err(AppError::from)
    }

    /// Encerramento: o controller é desligado por referência (as closures da UI só guardam `Weak`, então não há
    /// como o `Rc` ficar preso e o desligamento ser pulado) e solto em seguida, o que encerra o motor e as threads.
    pub fn finish(self) {
        CONTROLLER.with(|slot| slot.borrow_mut().take());
        match self.controller.try_borrow_mut() {
            Ok(mut c) => c.shutdown(),
            Err(_) => log::error!("controller ocupado no encerramento"),
        }
    }
}

/// Limite do cache de streams remotos em disco.
const STREAM_CACHE_BYTES: u64 = 256 * 1024 * 1024;

fn licenses_text() -> String {
    "Componentes usados no Kazzu\n\n\
     Slint, usado sob GPL-3.0: interface gráfica.\n\
     Inter (SIL OFL 1.1): fonte, incluída com o app.\n\
     Symphonia (MPL-2.0): decodificação de áudio. libopus (BSD-3-Clause): decodificador Opus.\n\
     cpal, rusqlite, lofty, image, ureq, rustls, rfd e demais bibliotecas: MIT ou Apache-2.0.\n\
     O YouTube Music é um serviço de terceiros; o Kazzu o acessa sem conta e não tem vínculo com ele.\n\n\
     A lista completa, com versões, está em docs/DEPENDENCIES.md."
        .to_owned()
}

/// Orçamento de um quadro (60 fps).
const FRAME_BUDGET: std::time::Duration = std::time::Duration::from_millis(16);
/// Janela em que os quadros lentos precisam dominar para o app oferecer "Reduzir transparência".
const SLOW_WINDOW: std::time::Duration = std::time::Duration::from_secs(2);

/// Mede o tempo de desenho de cada quadro (só local, nada sai do aparelho). Se por 2 s a maioria dos quadros estoura
/// 16 ms com o vidro ligado, avisa o controller UMA vez: ele oferece (não impõe) "Reduzir transparência".
fn watch_frames(window: &ui::AppWindow, sink: MsgSink) {
    use std::cell::Cell;
    use std::time::Instant;
    let started: Cell<Option<Instant>> = Cell::new(None);
    let window_start: Cell<Option<Instant>> = Cell::new(None);
    let (slow, total, offered) = (Cell::new(0u32), Cell::new(0u32), Cell::new(false));
    let result = window.window().set_rendering_notifier(move |state, _| match state {
        slint::RenderingState::BeforeRendering => started.set(Some(Instant::now())),
        slint::RenderingState::AfterRendering => {
            let (Some(t0), false) = (started.take(), offered.get()) else { return };
            let now = Instant::now();
            let w0 = *window_start.get().get_or_insert(now);
            total.set(total.get() + 1);
            if now - t0 > FRAME_BUDGET {
                slow.set(slow.get() + 1);
            }
            if now - w0 >= SLOW_WINDOW {
                // pelo menos 20 quadros na janela e 70% deles lentos: o aparelho não dá conta
                if total.get() >= 20 && slow.get() * 10 >= total.get() * 7 {
                    offered.set(true);
                    sink(Msg::SlowFrames);
                }
                window_start.set(Some(now));
                slow.set(0);
                total.set(0);
            }
        }
        _ => {}
    });
    if let Err(e) = result {
        log::info!("medição de quadros indisponível neste renderer: {e}");
    }
}

macro_rules! cb {
    ($ctrl:expr, |$c:ident $(, $arg:ident)*| $body:expr) => {{
        // `Weak`: a janela guarda estas closures; um `Rc` aqui manteria o controller vivo para sempre
        let ctrl = Rc::downgrade($ctrl);
        move |$($arg),*| {
            if let Some(rc) = ctrl.upgrade() {
                match rc.try_borrow_mut() {
                    Ok(mut $c) => {
                        $body;
                    }
                    Err(_) => log::error!("ação ignorada: controller ocupado"),
                }
            }
        }
    }};
}

fn tab_of(t: ui::Tab) -> NavTab {
    match t {
        ui::Tab::Home => NavTab::Home,
        ui::Tab::Explore => NavTab::Explore,
        ui::Tab::Library => NavTab::Library,
        ui::Tab::Search => NavTab::Search,
        ui::Tab::Settings => NavTab::Settings,
    }
}

/// Liga os callbacks de `AppState` ao controller.
fn wire(window: &ui::AppWindow, ctrl: &Rc<RefCell<Controller<SlintView>>>) {
    let s = window.global::<ui::AppState>();
    let weak = window.as_weak();

    // navegação (estado de tela vive no Slint; o controller só precisa saber a aba e se há detalhe)
    s.on_navigate({
        let ctrl = Rc::downgrade(ctrl);
        let weak = weak.clone();
        move |tab| {
            if let Some(w) = weak.upgrade() {
                let st = w.global::<ui::AppState>();
                st.set_tab(tab);
                st.set_detail_open(false);
            }
            if let Some(rc) = ctrl.upgrade()
                && let Ok(mut c) = rc.try_borrow_mut()
            {
                c.navigate(tab_of(tab));
            }
        }
    });
    s.on_open_player({
        let weak = weak.clone();
        move || {
            if let Some(w) = weak.upgrade() {
                w.global::<ui::AppState>().set_player_open(true);
            }
        }
    });
    s.on_close_player({
        let weak = weak.clone();
        move || {
            if let Some(w) = weak.upgrade() {
                w.global::<ui::AppState>().set_player_open(false);
            }
        }
    });
    s.on_close_detail({
        let ctrl = Rc::downgrade(ctrl);
        let weak = weak.clone();
        move || {
            if let Some(w) = weak.upgrade() {
                w.global::<ui::AppState>().set_detail_open(false);
            }
            if let Some(rc) = ctrl.upgrade()
                && let Ok(mut c) = rc.try_borrow_mut()
            {
                c.detail_closed();
            }
        }
    });

    // reprodução
    s.on_toggle_play(cb!(ctrl, |c| c.toggle_play()));
    s.on_next(cb!(ctrl, |c| c.next()));
    s.on_previous(cb!(ctrl, |c| c.previous()));
    s.on_seek(cb!(ctrl, |c, v| c.seek(v)));
    s.on_seek_relative(cb!(ctrl, |c, secs| c.seek_relative(i64::from(secs))));
    // volume: grava a configuração 1 s depois do último ajuste (arrastar o slider não grava a cada pixel)
    let volume_timer = Rc::new(slint::Timer::default());
    let save_later = {
        let ctrl = Rc::downgrade(ctrl);
        move |timer: &slint::Timer| {
            let ctrl = ctrl.clone();
            timer.start(slint::TimerMode::SingleShot, std::time::Duration::from_secs(1), move || {
                if let Some(rc) = ctrl.upgrade()
                    && let Ok(mut c) = rc.try_borrow_mut()
                {
                    c.flush_config();
                }
            });
        }
    };
    {
        let (set, t, later) = (cb!(ctrl, |c, v| c.set_volume(v)), volume_timer.clone(), save_later.clone());
        s.on_set_volume(move |v| {
            set(v);
            later(&t);
        });
        let (step, t, later) = (cb!(ctrl, |c, dir| c.volume_step(dir)), volume_timer, save_later);
        s.on_volume_step(move |d| {
            step(d);
            later(&t);
        });
    }
    s.on_toggle_shuffle(cb!(ctrl, |c| c.toggle_shuffle()));
    s.on_cycle_repeat(cb!(ctrl, |c| c.cycle_repeat()));
    s.on_toggle_like(cb!(ctrl, |c, id| c.toggle_like(&id)));
    let weak_window = weak.clone();
    let weak_ctrl = Rc::downgrade(ctrl);
    s.on_play_random_shortcut(move || {
        use slint::Model;
        use std::hash::{BuildHasher, RandomState};

        if let (Some(ui), Some(ctrl)) = (weak_window.upgrade(), weak_ctrl.upgrade()) {
            let state = ui.global::<ui::AppState>();
            let quick_picks = state.get_quick_picks();
            let tracks = if quick_picks.row_count() > 0 { quick_picks } else { state.get_recents() };
            let count = tracks.row_count();
            if count > 0 {
                let index = (RandomState::new().hash_one(count) % count as u64) as usize;
                if let Some(track) = tracks.row_data(index) {
                    ctrl.borrow_mut().play_track(&track.id);
                }
            }
        }
    });
    s.on_mobile_feature(cb!(ctrl, |c, feature| c.coming_soon(&feature)));
    s.on_play_track(cb!(ctrl, |c, id| c.play_track(&id)));
    s.on_play_next(cb!(ctrl, |c, id| c.queue_track(&id, QueueMode::PlayNext)));
    s.on_enqueue(cb!(ctrl, |c, id| c.queue_track(&id, QueueMode::Enqueue)));
    s.on_retry_track(cb!(ctrl, |c| c.retry_track()));
    s.on_open_lyrics(cb!(ctrl, |c| c.coming_soon("A letra das músicas")));
    s.on_open_output_picker(cb!(ctrl, |c| c.coming_soon("A troca de saída de áudio")));
    s.on_set_sleep_timer(cb!(ctrl, |c, _m| c.coming_soon("O timer de sono")));
    s.on_recognize_ambient(cb!(ctrl, |c| c.coming_soon("O reconhecimento de música pelo som ambiente")));
    s.on_connect_provider(cb!(ctrl, |c, _id| c.coming_soon("A conexão com outros provedores")));
    s.on_download_track(cb!(ctrl, |c, _id| c.coming_soon("Baixar músicas")));

    // Início e Explorar
    s.on_dismiss_first_run_banner(cb!(ctrl, |c| c.dismiss_first_run_banner()));
    s.on_retry_home(cb!(ctrl, |c| c.retry_home()));
    s.on_retry_explore(cb!(ctrl, |c| c.retry_explore()));
    s.on_open_item(cb!(ctrl, |c, kind, id| c.open_item(&kind, &id)));
    s.on_play_item(cb!(ctrl, |c, kind, id| c.play_item(&kind, &id)));

    // fila
    s.on_remember_focus(cb!(ctrl, |c, key| c.remember_focus(&key)));
    s.on_queue_remove(cb!(ctrl, |c, id| c.queue_remove(&id)));
    s.on_queue_jump(cb!(ctrl, |c, id| c.queue_jump(&id)));
    s.on_queue_move_up(cb!(ctrl, |c, id| c.queue_move_up(&id)));
    s.on_queue_clear(cb!(ctrl, |c| c.queue_clear()));

    // biblioteca, playlists e detalhe
    s.on_open_album(cb!(ctrl, |c, id| c.open_album(&id)));
    s.on_open_playlist(cb!(ctrl, |c, id| c.open_playlist(&id)));
    s.on_open_collection(cb!(ctrl, |c, id| c.open_collection(&id)));
    s.on_play_all(cb!(ctrl, |c| c.play_all(false)));
    s.on_shuffle_all(cb!(ctrl, |c| c.play_all(true)));
    s.on_retry_detail(cb!(ctrl, |c| c.retry_detail()));
    s.on_toggle_detail_saved(cb!(ctrl, |c| c.toggle_detail_saved()));
    s.on_goto_album(cb!(ctrl, |c, id| c.goto_album(&id)));
    s.on_goto_artist(cb!(ctrl, |c, id| c.goto_artist(&id)));
    use crate::worker::PlaylistEdit;
    s.on_rename_playlist(cb!(ctrl, |c, name| c.edit_playlist(PlaylistEdit::Rename(name.to_string()))));
    s.on_delete_playlist(cb!(ctrl, |c| c.edit_playlist(PlaylistEdit::Delete)));
    s.on_remove_playlist_entry(cb!(ctrl, |c, i, id| if let Ok(index) = usize::try_from(i) {
        c.edit_playlist(PlaylistEdit::Remove { index, track: id.to_string() });
    }));
    s.on_move_playlist_entry_up(cb!(ctrl, |c, i, id| if let Ok(index) = usize::try_from(i) {
        c.edit_playlist(PlaylistEdit::MoveUp { index, track: id.to_string() });
    }));
    s.on_library_order_changed(cb!(ctrl, |c, descending| c.library_order(descending)));
    s.on_create_playlist_named(cb!(ctrl, |c, name, track| c.create_playlist(&name, &track)));
    s.on_add_to_playlist(cb!(ctrl, |c, track, playlist| c.add_to_playlist(&track, &playlist)));
    s.on_add_folder(cb!(ctrl, |c| c.add_folder()));
    s.on_browse_folders(cb!(ctrl, |c, path| c.browse_folders(&path)));
    s.on_add_named_folder(cb!(ctrl, |c, path| c.add_named_folder(&path)));
    s.on_import_account(cb!(ctrl, |c, provider, token| c.import_account(provider.to_string(), token.to_string())));
    s.on_remove_folder(cb!(ctrl, |c, path| c.remove_folder(&path)));
    s.on_rescan(cb!(ctrl, |c| c.rescan()));

    // busca
    s.on_search(cb!(ctrl, |c, text| c.search(&text)));
    s.on_search_source_changed(cb!(ctrl, |c, source| c.search_source_changed(source)));
    s.on_search_kind_changed(cb!(ctrl, |c, kind| c.search_kind_changed(kind)));
    s.on_clear_recent_searches(cb!(ctrl, |c| c.clear_recent_searches()));
    s.on_remove_recent_search(cb!(ctrl, |c, text| c.remove_recent_search(&text)));

    // configurações
    s.on_settings_match(|query, words| settings_match(&query, &words));
    s.on_palette_changed(cb!(ctrl, |c, palette| c.set_color_palette(palette)));
    s.on_visual_style_changed(cb!(ctrl, |c, style, artwork| c.set_visual_style(style, artwork)));
    s.on_library_layout_changed(cb!(ctrl, |c, view, density| c.set_library_layout(view, density)));
    s.on_theme_mode_changed(cb!(ctrl, |c, mode| c.set_theme(match mode {
        ui::ThemeMode::Auto => ThemePref::Auto,
        ui::ThemeMode::Light => ThemePref::Light,
        ui::ThemeMode::Dark => ThemePref::Dark,
        ui::ThemeMode::Oled => ThemePref::Oled,
    })));
    s.on_stream_quality_changed(cb!(ctrl, |c, i| c.set_stream_quality(StreamQualityPref::from_index(i))));
    s.on_autoplay_radio_changed(cb!(ctrl, |c, on| c.set_autoplay_radio(on)));
    s.on_reduce_transparency_changed(cb!(ctrl, |c, on| c.set_reduce_transparency(on)));
    s.on_reduce_motion_changed(cb!(ctrl, |c, on| c.set_reduce_motion(on)));
    s.on_perf_offer_accept(cb!(ctrl, |c| c.perf_offer_accept()));
    s.on_perf_offer_dismiss(cb!(ctrl, |c| c.perf_offer_dismiss()));
    s.on_clear_history(cb!(ctrl, |c| c.clear_history()));
    s.on_report_problem(cb!(ctrl, |c| c.report_problem()));
    s.on_toast_action(cb!(ctrl, |c| c.toast_action()));
    s.on_dismiss_toast(|| {});
}

/// Accent-insensitive matching for the small native settings index.
pub fn settings_match(query: &str, words: &str) -> bool {
    fn normalize(s: &str) -> String {
        s.to_lowercase().chars().map(|c| match c {
            'á' | 'à' | 'â' | 'ã' => 'a', 'é' | 'ê' => 'e', 'í' => 'i',
            'ó' | 'ô' | 'õ' => 'o', 'ú' | 'ü' => 'u', 'ç' => 'c', _ => c,
        }).collect()
    }
    let words = normalize(words);
    normalize(query).split_whitespace().all(|term| words.contains(term))
}
