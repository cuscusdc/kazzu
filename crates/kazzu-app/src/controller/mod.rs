//! Controller: liga motor de áudio, biblioteca, provedores remotos e tela. Não conhece o Slint (fala com a tela pelo
//! trait `View`), então é testável sem janela. Tudo aqui roda na thread de UI e **nunca bloqueia**: o trabalho pesado
//! (disco, rede, decodificação) vai para `JobRunner`; os resultados voltam como `Msg`.
//!
//! - `mod.rs`: estado, mensagens, motor de áudio, comandos locais, preferências.
//! - `remote.rs`: YouTube Music (Início, Explorar, busca, detalhes, fila JIT com rádio, URL vencida, curtidas).

mod remote;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::{Duration, Instant};

use kazzu_audio::{EngineHandle, StreamEvent};
use kazzu_core::{CoverRef, OutputInfo, PlaybackEvent, PlaybackStatus, QueueItem, QueueItemId, RepeatMode, Track as CoreTrack, TrackSource};
use kazzu_ytm::Category;

use crate::accent::Accent;
use crate::art::Pixels;
use crate::config::{AccentPref, Config, StreamQualityPref, ThemePref};
use crate::format;
use crate::lru::Lru;
use crate::remote::RemoteRef;
use crate::view::View;
use crate::vm::*;
use crate::worker::{DetailRef, Job, JobResult, JobRunner, QueueMode, Scope};
use crate::ytm_vm;

pub use remote::Entry;
use remote::{Context, EngineRadio, ToastAction};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NavTab {
    Home,
    Explore,
    Library,
    Search,
    Settings,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum DetailRoute {
    Library(DetailRef),
    Catalog(RemoteRef),
}

#[derive(Clone)]
struct PageVisit {
    route: DetailRoute,
    scroll: f32,
    focus: String,
}

/// Preferências lidas do sistema operacional na inicialização.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SystemPrefs {
    pub reduce_motion: Option<bool>,
}

/// Tudo que chega ao controller vindo de outras threads.
pub enum Msg {
    Job(JobResult),
    Engine(PlaybackEvent),
    /// Eventos do streaming HTTP (URL vencida, rede caiu).
    Stream(StreamEvent),
    FolderPicked(Option<PathBuf>),
    #[cfg(any(target_os = "android", target_os = "ios"))]
    StreamCacheCleared(std::io::Result<u64>),
    #[cfg(any(target_os = "android", target_os = "ios"))]
    StreamRemoved(std::io::Result<bool>),
    #[cfg(any(target_os = "android", target_os = "ios"))]
    StorageUsage { files: u64, cache: u64 },
    AccountFetched(Result<(Vec<crate::accounts::Import>, usize), String>),
    System(SystemPrefs),
    /// Quadros acima do orçamento por tempo demais: oferece "Reduzir transparência".
    SlowFrames,
    RetryEngineRadio(u64),
}

pub type MsgSink = Arc<dyn Fn(Msg) + Send + Sync>;

/// Escuta de pelo menos 30 s (ou metade da faixa) conta como "ouvida" no histórico.
const LISTEN_MIN: Duration = Duration::from_secs(30);
const SEEK_STEP_SECS: i64 = 5;
const VOLUME_STEP: f32 = 0.05;
/// Quantas próximas faixas a fila da UI mostra (o resto vira "e mais N"): fila de milhares não reconstrói milhares de linhas.
pub const QUEUE_VIEW_MAX: usize = 100;
/// View-models de faixas da fila guardados (LRU).
const VM_CACHE_MAX: usize = if cfg!(any(target_os = "android", target_os = "ios")) { 240 } else { 600 };
/// Faixas remotas lembradas para poder tocá-las pelo id da linha.
const REMOTE_TRACKS_MAX: usize = 4000;
/// Capas remotas decodificadas guardadas em memória.
const COVER_CACHE_MAX: usize = if cfg!(any(target_os = "android", target_os = "ios")) { 160 } else { 500 };
/// Depois disto, uma capa pedida e ainda ausente (falhou, ou saiu do cache) pode ser pedida de novo.
pub(crate) const COVER_RETRY: Duration = Duration::from_secs(30);

const NOTE_AUTOPLAY: &str = "Quando a fila acabar, o Kazzu continua com músicas parecidas do YouTube Music. Dá para desligar em Ajustes.";

/// Número de ponto flutuante utilizável como fração/volume: finito, preso a 0..=1; NaN e infinitos viram `fallback`.
pub fn sane_unit(v: f32, fallback: f32) -> f32 {
    if v.is_finite() { v.clamp(0.0, 1.0) } else { fallback }
}

/// Texto curto para o usuário; o detalhe técnico vai para o log.
fn friendly_error(message: &str) -> String {
    let m = message.to_ascii_lowercase();
    if m.contains("audio output") || m.contains("alsa") || m.contains("device") {
        "A saída de áudio não respondeu. Confira o dispositivo nas configurações do sistema.".to_owned()
    } else if m.contains("unsupported") || m.contains("codec") || m.contains("format") {
        "O formato deste arquivo não é suportado.".to_owned()
    } else if m.contains("no such file") || m.contains("not found") || m.contains("os error 2") {
        "O arquivo não está mais na pasta.".to_owned()
    } else if m.contains("network") || m.contains("http") || m.contains("stream") || m.contains("timed out") {
        "A conexão falhou. Tente de novo em instantes.".to_owned()
    } else {
        message.to_owned()
    }
}

pub struct Controller<V: View> {
    view: V,
    engine: EngineHandle,
    jobs: Box<dyn JobRunner>,
    #[cfg_attr(target_os = "android", allow(dead_code))] // no Android o seletor de pastas (único uso) não existe
    sink: MsgSink,
    cfg: Config,
    cfg_path: Option<PathBuf>,
    data_dir: Option<PathBuf>,
    #[cfg(all(unix, not(target_os = "android")))]
    discord: Option<crate::discord_presence::DiscordPresence>,
    #[cfg(any(target_os = "android", target_os = "ios"))]
    stats_seq: u64,
    #[cfg(any(target_os = "android", target_os = "ios"))]
    cache_dir: Option<PathBuf>,
    #[cfg(any(target_os = "android", target_os = "ios"))]
    following_room: bool,
    #[cfg(any(target_os = "android", target_os = "ios"))]
    followed_id: Option<String>,

    tab: NavTab,
    detail_open: bool,
    detail_ids: Vec<String>,
    detail_title: String,
    detail_vm: DetailVm,
    detail_what: Option<RemoteRef>,
    detail_album: Option<kazzu_core::Album>,
    recents_ids: Vec<String>,

    pb: PlaybackVm,
    queue_items: Vec<QueueItem>,
    current: Option<usize>,
    position: Duration,
    duration: Option<Duration>,
    queue_seq: u64,
    vm_cache: Lru<String, TrackVm>,
    detail_seq: u64,
    visits: HashMap<NavTab, Vec<PageVisit>>,
    section_focus: HashMap<NavTab, String>,
    cfg_dirty: bool,
    art_seq: u64,
    now_art: NowArtVm,
    recorded_current: bool,
    listen_sample: Option<(Duration, Instant)>,
    listened: Duration,
    #[cfg(any(target_os = "android", target_os = "ios"))]
    listen_started: Option<i64>,

    liked: HashSet<String>,
    /// Curtidas alteradas aqui e ainda não confirmadas (uma atualização atrasada não pode desfazê-las).
    pending_likes: HashMap<String, bool>,
    home: HomeVm,
    library: LibraryVm,
    library_descending: bool,
    settings: SettingsVm,
    scanning: bool,
    auto_scanned: bool,

    search_query: String,
    search_seq: u64,
    search_local_songs: Vec<TrackVm>,
    search_local_albums: Vec<AlbumVm>,
    search_local_ids: Vec<String>,
    search_local_pending: bool,
    search_remote: Option<SearchVm>,
    search_remote_failed: Option<crate::catalog::ErrorKind>,
    search_suggestions: Vec<String>,
    search_kind: i32,
    search_source: i32,
    account_pending: bool,
    folder_list_seq: u64,

    // ---- remoto (YouTube Music) ----
    remote_enabled: bool,
    engine_quality: Option<Arc<AtomicI32>>,
    engine_radio: Option<EngineRadio>,
    online: bool,
    home_seq: u64,
    explore: ExploreVm,
    explore_seq: u64,
    explore_requested: bool,
    categories: Vec<Category>,
    remote_tracks: Lru<String, CoreTrack>,
    covers: Lru<String, Pixels>,
    /// Capas já pedidas e quando: pedido velho sem a imagem em memória (falhou ou foi descartada) pode ser repetido.
    covers_requested: HashMap<String, Instant>,
    /// Telas que ganharam capas enquanto não estavam à vista: refeitas ao navegar até elas.
    stale_screens: HashSet<NavTab>,
    cover_retry: Duration,
    ctx: Option<Context>,
    ctx_seq: u64,
    quality_labels: HashMap<QueueItemId, String>,
    toast_action: ToastAction,
    /// A rede falhou no meio da faixa e o motor espera `play()` para tentar de novo.
    stalled: bool,
    /// Renovações de URL vencida por faixa (limite: 2 em 60 s).
    expired_log: HashMap<QueueItemId, Vec<Instant>>,
    perf_dismissed: bool,
    queue_vm: QueueVm,
}

impl<V: View> Controller<V> {
    pub fn new(view: V, engine: EngineHandle, jobs: Box<dyn JobRunner>, sink: MsgSink, cfg: Config, cfg_path: Option<PathBuf>) -> Self {
        let volume = sane_unit(cfg.volume, 1.0);
        Self {
            view,
            engine,
            jobs,
            sink,
            cfg,
            cfg_path,
            data_dir: None,
            #[cfg(all(unix, not(target_os = "android")))]
            discord: None,
            #[cfg(any(target_os = "android", target_os = "ios"))]
            stats_seq: 0,
            #[cfg(any(target_os = "android", target_os = "ios"))]
            cache_dir: None,
            #[cfg(any(target_os = "android", target_os = "ios"))]
            following_room: false,
            #[cfg(any(target_os = "android", target_os = "ios"))]
            followed_id: None,
            tab: NavTab::Home,
            detail_open: false,
            detail_ids: vec![],
            detail_title: String::new(),
            detail_vm: DetailVm::default(),
            detail_what: None,
            detail_album: None,
            recents_ids: vec![],
            pb: PlaybackVm { volume, output_name: "Padrão do sistema".into(), bit_perfect: true, error_action: "Tentar de novo".into(), ..Default::default() },
            queue_items: vec![],
            current: None,
            position: Duration::ZERO,
            duration: None,
            queue_seq: 0,
            vm_cache: Lru::new(VM_CACHE_MAX),
            detail_seq: 0,
            visits: HashMap::new(),
            section_focus: HashMap::new(),
            cfg_dirty: false,
            art_seq: 0,
            now_art: NowArtVm::default(),
            recorded_current: false,
            listen_sample: None,
            listened: Duration::ZERO,
            #[cfg(any(target_os = "android", target_os = "ios"))]
            listen_started: None,
            liked: HashSet::new(),
            pending_likes: HashMap::new(),
            home: HomeVm::default(),
            library: LibraryVm::default(),
            library_descending: false,
            settings: SettingsVm::default(),
            scanning: false,
            auto_scanned: false,
            search_query: String::new(),
            search_seq: 0,
            search_local_songs: vec![],
            search_local_albums: vec![],
            search_local_ids: vec![],
            search_local_pending: false,
            search_remote: None,
            search_remote_failed: None,
            search_suggestions: vec![],
            search_kind: 0,
            search_source: 0,
            account_pending: false,
            folder_list_seq: 0,
            remote_enabled: false,
            engine_quality: None,
            engine_radio: None,
            online: true,
            home_seq: 0,
            explore: ExploreVm::default(),
            explore_seq: 0,
            explore_requested: false,
            categories: vec![],
            remote_tracks: Lru::new(REMOTE_TRACKS_MAX),
            covers: Lru::new(COVER_CACHE_MAX),
            covers_requested: HashMap::new(),
            stale_screens: HashSet::new(),
            cover_retry: COVER_RETRY,
            ctx: None,
            ctx_seq: 0,
            quality_labels: HashMap::new(),
            toast_action: ToastAction::None,
            stalled: false,
            expired_log: HashMap::new(),
            perf_dismissed: false,
            queue_vm: QueueVm::default(),
        }
    }

    /// Liga o provedor remoto padrão (YouTube Music): o Início passa a carregar o feed ao abrir.
    pub fn enable_remote(&mut self) {
        self.remote_enabled = true;
    }

    pub(crate) fn enable_engine_remote(&mut self, quality: Arc<AtomicI32>) {
        self.enable_remote();
        self.engine_quality = Some(quality);
    }

    /// Intervalo mínimo para pedir de novo uma capa que não chegou (padrão 30 s; testes encurtam).
    #[doc(hidden)]
    pub fn set_cover_retry(&mut self, d: Duration) {
        self.cover_retry = d;
    }

    #[cfg(all(unix, not(target_os = "android")))]
    pub(crate) fn enable_discord_from_env(&mut self) {
        self.discord = crate::discord_presence::DiscordPresence::from_env();
    }

    #[cfg(all(unix, not(target_os = "android")))]
    fn publish_discord(&self) {
        if let Some(discord) = &self.discord {
            discord.update(self.pb.now.as_ref().filter(|_| self.pb.playing));
        }
    }

    /// Onde gravar o arquivo de "Relatar um problema".
    pub fn set_data_dir(&mut self, dir: PathBuf) {
        self.data_dir = Some(dir);
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) fn set_cache_dir(&mut self, dir: PathBuf) {
        self.cache_dir = Some(dir);
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    pub fn playback(&self) -> &PlaybackVm {
        &self.pb
    }

    pub fn engine(&self) -> &EngineHandle {
        &self.engine
    }

    /// Primeiro quadro: aplica preferências e pede os dados (biblioteca local e, se ativo, o feed do Início).
    pub fn init(&mut self, system: SystemPrefs) {
        if self.cfg.reduce_motion.is_none() {
            self.cfg.reduce_motion = system.reduce_motion;
        }
        self.push_prefs();
        self.push_accent();
        self.view.set_playback(&self.pb);
        self.view.set_volume(self.pb.volume);
        self.view.set_loading(true);
        self.view.set_online(true);
        self.publish_search();
        self.jobs.submit(Job::Refresh);
        if self.remote_enabled {
            self.home.state = LoadStateVm::Loading;
            self.request_home(HomeSectionVm::ForYou);
            self.request_explore();
        } else {
            self.home.state = LoadStateVm::Ready;
            // sem YouTube Music não há feed nem Explorar: o Início mostra a música do aparelho
            self.explore.state = LoadStateVm::Empty;
            self.publish_explore();
        }
        self.publish_home();
    }

    // ----------------------------------------------------------------------------------------
    // Mensagens de outras threads
    // ----------------------------------------------------------------------------------------

    fn finish_import(&mut self, result: Result<String, String>) {
        self.account_pending = false;
        let (message, error) = match result {
            Ok(message) => (message, false),
            Err(message) => (message, true),
        };
        self.view.import_status(false, &message);
        self.toast(&message, error);
    }

    fn sort_library(&mut self) {
        self.library.albums.sort_by_cached_key(|a| a.title.to_lowercase());
        self.library.artists.sort_by_cached_key(|a| a.name.to_lowercase());
        self.library.playlists.sort_by_cached_key(|p| p.name.to_lowercase());
        if self.library_descending {
            self.library.albums.reverse();
            self.library.artists.reverse();
            self.library.playlists.reverse();
        }
    }

    pub fn library_order(&mut self, descending: bool) {
        self.library_descending = descending;
        self.sort_library();
        self.view.set_library(&self.library);
    }

    pub fn edit_playlist(&mut self, edit: crate::worker::PlaylistEdit) {
        let id = self.detail_vm.editable_playlist.clone();
        if self.detail_open && !id.is_empty() {
            self.jobs.submit(Job::EditPlaylist { id, edit });
        }
    }

    pub fn import_account(&mut self, provider: String, token: String) {
        if self.account_pending {
            self.toast("Já existe uma importação em andamento.", false);
            return;
        }
        let token = token.trim().to_owned();
        if token.is_empty() || token.len() > 8192 || token.bytes().any(|b| !b.is_ascii_graphic()) {
            self.finish_import(Err("Informe um token de acesso OAuth válido.".into()));
            return;
        }
        let sink = self.sink.clone();
        if std::thread::Builder::new().name("kazzu-account".into()).spawn(move || {
            sink(Msg::AccountFetched(crate::accounts::fetch(&provider, &token)));
        }).is_ok() {
            self.account_pending = true;
            self.view.import_status(true, "Importando coleções…");
        } else {
            self.finish_import(Err("Não foi possível iniciar a importação.".into()));
        }
    }

    pub fn handle(&mut self, msg: Msg) {
        match msg {
            #[cfg(any(target_os = "android", target_os = "ios"))]
            Msg::StreamRemoved(result) => {
                match result {
                    Ok(true) => self.notice("Cópia em cache removida.", false),
                    Ok(false) => self.notice("A cópia já saiu do cache.", false),
                    Err(_) => self.notice("Não foi possível remover a cópia.", true),
                }
                if self.detail_open && self.detail_vm.cached_audio {
                    self.load_route(DetailRoute::Library(DetailRef::Downloads));
                }
                self.android_storage_usage();
            }
            #[cfg(any(target_os = "android", target_os = "ios"))]
            Msg::StorageUsage { files, cache } => self.view.set_storage(files, cache),
            #[cfg(any(target_os = "android", target_os = "ios"))]
            Msg::StreamCacheCleared(result) => {
                match result {
                    Ok(bytes) => self.notice(&format!("Cache de áudio limpo. {} MiB liberados.", bytes / 1_048_576), false),
                    Err(_) => self.notice("Não foi possível limpar todo o cache de áudio.", true),
                }
                self.android_storage_usage();
            }

            Msg::RetryEngineRadio(seq) => self.retry_engine_radio(seq),
            Msg::Engine(ev) => self.on_engine(ev),
            Msg::Stream(ev) => self.on_stream(ev),
            Msg::Job(r) => self.on_job(r),
            Msg::FolderPicked(Some(path)) => self.jobs.submit(Job::AddFolder(path)),
            Msg::FolderPicked(None) => {}
            Msg::AccountFetched(result) => {
                match result {
                    Ok((lists, skipped)) => self.jobs.submit(Job::ImportAccount { lists, skipped }),
                    Err(error) => self.finish_import(Err(error)),
                }
            }
            Msg::System(sys) => {
                if self.cfg.reduce_motion.is_none() {
                    self.cfg.reduce_motion = sys.reduce_motion;
                    self.push_prefs();
                }
            }
            Msg::SlowFrames => {
                if self.cfg.reduce_transparency != Some(true) && !self.perf_dismissed {
                    self.view.set_perf_offer(true);
                }
            }
        }
    }

    fn on_job(&mut self, r: JobResult) {
        match r {
            JobResult::Refreshed(snap) => {
                let mut snap = *snap;
                // abertura: reindexa (incremental) as pastas já configuradas
                if !self.auto_scanned && !snap.settings.folders.is_empty() {
                    self.auto_scanned = true;
                    self.jobs.submit(Job::Scan);
                }
                for t in snap.remote_liked {
                    self.remember_remote(&t);
                }
                self.liked = snap.liked;
                for (id, liked) in &self.pending_likes {
                    if *liked { self.liked.insert(id.clone()); } else { self.liked.remove(id); }
                }
                self.flag_tracks(&mut snap.home.recents);
                self.recents_ids = snap.home.recents.iter().map(|t| t.id.clone()).collect();
                self.home.recents = snap.home.recents;
                if !self.remote_enabled {
                    self.home.albums = snap.home.albums;
                }
                self.library = snap.library;
                self.sort_library();
                self.settings = snap.settings;
                self.view.set_loading(false);
                self.publish_home();
                self.view.set_library(&self.library);
                self.publish_settings();
                if self.pb.now.is_none() {
                    self.view.set_ambient(snap.ambient.as_ref());
                }
            }
            JobResult::PlaylistEdited { id, deleted } => {
                let current = self.detail_open && self.detail_vm.editable_playlist == id;
                let route = DetailRoute::Library(DetailRef::Playlist(id));
                if deleted {
                    for pages in self.visits.values_mut() {
                        pages.retain(|p| p.route != route);
                    }
                }
                if current {
                    if deleted {
                        self.detail_open = false;
                        self.detail_seq += 1;
                        self.view.open_detail(false);
                    }
                    self.restore_visit();
                }
            }
            JobResult::AccountImported(result) => self.finish_import(result),
            JobResult::DetailFailed { seq, message } => {
                if seq == self.detail_seq {
                    self.show_detail(DetailVm { state: LoadStateVm::Error, error_text: message, ..Default::default() });
                }
            }
            JobResult::ScanProgress(p) => {
                self.scanning = true;
                self.view.set_scan(Some(&p));
            }
            JobResult::ScanDone(res) => {
                self.scanning = false;
                self.view.set_scan(None);
                match res {
                    Ok(report) => {
                        if !report.failed.is_empty() {
                            let n = report.failed.len();
                            let text = if n == 1 { "1 arquivo não pôde ser lido.".to_owned() } else { format!("{} arquivos não puderam ser lidos.", format::thousands(n as u64)) };
                            self.toast(&text, true);
                        }
                        self.jobs.submit(Job::Refresh);
                    }
                    Err(e) => self.toast(&format!("A leitura da biblioteca falhou. {e}"), true),
                }
            }
            JobResult::FoldersChanged => {
                self.jobs.submit(Job::Scan);
                self.jobs.submit(Job::Refresh);
            }
            JobResult::Search { seq, songs, albums, ids } => {
                if seq == self.search_seq {
                    self.search_local_songs = songs;
                    self.search_local_albums = albums;
                    self.search_local_ids = ids;
                    self.search_local_pending = false;
                    self.publish_search();
                }
            }
            JobResult::Queue { items, start, shuffle, label } => {
                // fila gerenciada pelo motor (tudo local): o contexto remoto, se houver, acabou
                self.ctx = None;
                self.engine_radio = None;
                self.stalled = false;
                self.pb.origin = label;
                self.pb.error.clear();
                // O embaralhar escolhido pelo usuário vale para a nova fila (o motor reembaralha no Load). Só o
                // pedido explícito ("Tudo em ordem aleatória", "Aleatório") liga o modo.
                self.engine.load(items, start);
                if shuffle && !self.pb.shuffle {
                    self.engine.set_shuffle(true);
                }
                self.view.set_playback(&self.pb);
            }
            JobResult::PreparedQueue { seq, items, start, shuffle, label } => {
                if seq != self.ctx_seq {
                    return;
                }
                if items.is_empty() {
                    self.notice("Não há músicas para tocar aqui.", true);
                    return;
                }
                self.ctx = None;
                self.stalled = false;
                self.pb.origin = label;
                self.pb.error.clear();
                self.install_engine_radio(seq, &items, start);
                self.engine.load(items, start);
                if shuffle && !self.pb.shuffle {
                    self.engine.set_shuffle(true);
                }
                self.view.set_playback(&self.pb);
            }
            JobResult::QueueItems { items, mode } => self.queued(items, mode == QueueMode::PlayNext),
            JobResult::Detail { seq, detail, ids, remote_tracks } => {
                // resposta atrasada (o usuário já trocou de aba ou abriu outra coisa): descarta
                if seq != self.detail_seq {
                    return;
                }
                for t in &remote_tracks {
                    self.remember_remote(t);
                }
                let mut vm = *detail;
                self.flag_tracks(&mut vm.tracks);
                self.detail_ids = ids;
                self.detail_title = vm.title.clone();
                self.detail_what = None;
                self.detail_album = None;
                self.show_detail(vm);
                self.restore_detail_scroll();
                self.view.open_detail(true);
                self.detail_open = true;
            }
            #[cfg(any(target_os = "android", target_os = "ios"))]
            JobResult::Lyrics { seq, lines } if seq == self.art_seq => self.view.set_lyrics(&lines),
            #[cfg(any(target_os = "android", target_os = "ios"))]
            JobResult::Lyrics { .. } => {}
            #[cfg(any(target_os = "android", target_os = "ios"))]
            JobResult::ListeningStats { seq, stats, export } => {
                if export {
                    self.android_export_stats(&stats);
                } else if seq == self.stats_seq {
                    self.view.set_listening_stats(&stats);
                }
            }
            #[cfg(any(target_os = "android", target_os = "ios"))]
            JobResult::LocalTagsEdited(result) => match result {
                Ok(()) => {
                    self.notice("Metadados salvos na cópia local.", false);
                    self.rescan();
                }
                Err(error) => self.notice(&error, true),
            },
            #[cfg(any(target_os = "android", target_os = "ios"))]
            JobResult::M3uImported { count, skipped } => {
                self.notice(&format!("{count} faixas importadas. {skipped} sem correspondência única."), false);
                self.jobs.submit(Job::Refresh);
            }
            #[cfg(any(target_os = "android", target_os = "ios"))]
            JobResult::BackupReady { result, lan, room } => match result {
                Ok(json) if !lan && crate::android::export_backup(&json) => {}
                Ok(json) if lan && room && crate::android::publish_room_backup(&json) => {}
                Ok(json) if lan && crate::android::host_backup(&json) => {}
                Ok(_) => self.notice("Não foi possível iniciar o compartilhamento do backup.", true),
                Err(error) => self.notice(&error, true),
            },
            #[cfg(any(target_os = "android", target_os = "ios"))]
            JobResult::PeerSynced(result) => match result {
                Ok(0) => {}
                Ok(_) => self.jobs.submit(Job::Refresh),
                Err(error) => log::warn!("sincronização LAN: {error}"),
            },
            #[cfg(any(target_os = "android", target_os = "ios"))]
            JobResult::BackupRestored(result) => match result {
                Ok(backup) => {
                    self.cfg = backup.config;
                    self.engine.set_crossfade(Duration::from_secs(self.cfg.mobile_appearance.crossfade_seconds as u64));
                    self.set_volume(self.cfg.volume);
                    self.set_stream_quality(self.cfg.stream_quality);
                    self.push_prefs();
                    self.push_accent();
                    self.jobs.submit(Job::Refresh);
                    self.notice(&format!("{} playlists, {} curtidas e {} itens de histórico restaurados. {} faixas não encontradas.", backup.playlists, backup.liked, backup.history, backup.missing), false);
                }
                Err(error) => self.notice(&error, true),
            },
            #[cfg(any(target_os = "android", target_os = "ios"))]
            JobResult::RemoteLyrics { seq, result } => {
                if seq == self.art_seq {
                    match result {
                        Ok(Some(lyrics)) => self.view.set_remote_lyrics(&lyrics.text, lyrics.source.as_deref().unwrap_or_default(), false),
                        Ok(None) => self.view.set_remote_lyrics("", "", false),
                        Err(_) => self.view.set_remote_lyrics("", "Não foi possível buscar a letra.", false),
                    }
                }
            }
            JobResult::NowArt { seq, art } => {
                if seq == self.art_seq {
                    self.now_art = *art;
                    if let Some(now) = self.pb.now.as_mut() {
                        now.cover = self.now_art.thumb.clone();
                    }
                    self.view.set_playback(&self.pb);
                    self.view.set_now_art(&self.now_art);
                    self.view.set_ambient(self.now_art.backdrop.as_ref());
                    self.push_accent();
                }
            }
            JobResult::TrackVms { seq, tracks } => {
                // o cache aproveita qualquer resposta; só publicar depende de ainda ser a fila atual
                for t in tracks {
                    self.vm_cache.insert(t.id.clone(), t);
                }
                if seq == self.queue_seq {
                    self.publish_queue();
                }
            }
            JobResult::Recents(mut list) => {
                self.flag_tracks(&mut list);
                self.recents_ids = list.iter().map(|t| t.id.clone()).collect();
                self.home.recents = list;
                self.publish_home();
            }
            JobResult::FolderListing { seq, result } => {
                if seq == self.folder_list_seq {
                    self.view.set_folder_listing(result.as_ref().map_err(String::as_str));
                }
            }
            JobResult::ListeningSummary(summary) => {
                self.library.listening_summary = summary;
                self.view.set_library(&self.library);
            }
            JobResult::Failed(msg) => self.toast(&msg, true),
            JobResult::Notice(msg) => self.toast(&msg, false),
            JobResult::LikedSaved { id, liked } => {
                if self.pending_likes.get(&id) == Some(&liked) {
                    self.pending_likes.remove(&id);
                }
            }
            JobResult::Remote(r) => self.on_remote(r),
        }
    }

    /// Aviso sem ação.
    fn toast(&mut self, text: &str, error: bool) {
        self.toast_action = ToastAction::None;
        self.view.toast(text, error, "");
    }

    /// Aviso com botão ("Desfazer", "Tentar de novo"); a ação roda em [`Controller::toast_action`].
    fn toast_with(&mut self, text: &str, error: bool, label: &str, action: ToastAction) {
        self.toast_action = action;
        self.view.toast(text, error, label);
    }

    // ----------------------------------------------------------------------------------------
    // Eventos do motor de áudio
    // ----------------------------------------------------------------------------------------

    pub fn on_engine(&mut self, ev: PlaybackEvent) {
        match ev {
            PlaybackEvent::StatusChanged { status } => {
                if status != PlaybackStatus::Playing {
                    self.listen_sample = None;
                }
                self.pb.playing = status == PlaybackStatus::Playing;
                self.pb.loading = status == PlaybackStatus::Buffering;
                if self.pb.playing {
                    self.stalled = false;
                    if !self.pb.error.is_empty() {
                        self.pb.error.clear();
                    }
                }
                self.view.set_playback(&self.pb);
                #[cfg(all(unix, not(target_os = "android")))]
                self.publish_discord();
            }
            PlaybackEvent::TrackChanged { item, duration } => {
                self.position = Duration::ZERO;
                self.duration = duration;
                self.recorded_current = false;
                self.listen_sample = None;
                self.listened = Duration::ZERO;
                #[cfg(any(target_os = "android", target_os = "ios"))]
                {
                    self.listen_started = None;
                }
                self.record_listen_if_needed();
                self.pb.error.clear();
                match item {
                    Some(item) => {
                        self.on_track_changed(&item);
                    }
                    None => {
                        self.art_seq += 1;
                        self.pb.now = None;
                        self.pb.has_now = false;
                        self.pb.quality_label.clear();
                        self.pb.source.clear();
                        self.now_art = NowArtVm::default();
                        self.view.set_now_art(&self.now_art);
                        self.push_accent();
                    }
                }
                self.view.set_playback(&self.pb);
                self.view.mark_playing(self.pb.now.as_ref().map(|t| t.id.as_str()));
                #[cfg(all(unix, not(target_os = "android")))]
                self.publish_discord();
                self.push_progress();
                self.unstall(true);
            }
            PlaybackEvent::Position { position, duration } => {
                if self.pb.playing {
                    #[cfg(any(target_os = "android", target_os = "ios"))]
                    if self.listen_started.is_none() {
                        self.listen_started = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
                            .ok().map(|elapsed| elapsed.as_secs() as i64);
                    }
                    let now = Instant::now();
                    if let Some((previous, sampled)) = self.listen_sample.replace((position, now))
                        && let Some(delta) = position.checked_sub(previous)
                        && delta <= now.saturating_duration_since(sampled) + Duration::from_millis(250)
                    {
                        self.listened = self.listened.saturating_add(delta);
                    }
                }
                self.position = position;
                if duration.is_some() {
                    self.duration = duration;
                }
                self.push_progress();
                // fila parada por falha de rede: tenta de novo quando o recuo vence
                self.unstall(false);
            }
            PlaybackEvent::TrackEnded { id } => {
                if let Some(item) = self.queue_items.iter().find(|i| i.id == id) {
                    let ms = item.track.duration.map_or(0, |d| d.as_millis() as u64);
                    // o histórico (e a retrospectiva) é só de faixas do aparelho
                    if matches!(item.track.source, TrackSource::Local { .. }) {
                        self.jobs.submit(Job::RecordPlay { id: item.track.id.to_string(), listened_ms: ms });
                    }
                    // não marca recorded_current aqui: o scrobble do Last.fm acontece em
                    // record_listen_if_needed() quando TrackChanged chega, independentemente da ordem
                    // em que TrackEnded e TrackChanged são emitidos pelo motor.
                }
            }
            PlaybackEvent::QueueChanged { items, current } => {
                self.queue_items = items;
                self.current = current;
                self.queue_seq += 1;
                let missing: Vec<String> = self.upcoming().iter().take(QUEUE_VIEW_MAX).filter(|i| matches!(i.track.source, TrackSource::Local { .. })).map(|i| i.track.id.to_string()).filter(|id| !self.vm_cache.contains(id)).collect();
                if !missing.is_empty() {
                    self.jobs.submit(Job::TrackVms { seq: self.queue_seq, ids: missing });
                }
                // mostra já o que se sabe (sem capas) para a fila não ficar defasada
                self.publish_queue();
                self.ensure_lookahead();
            }
            PlaybackEvent::VolumeChanged { volume } => {
                let volume = sane_unit(volume, self.pb.volume);
                self.pb.volume = volume;
                self.cfg.volume = volume;
                self.cfg_dirty = true;
                self.view.set_volume(volume);
            }
            PlaybackEvent::RepeatChanged { mode } => {
                self.pb.repeat = match mode {
                    // numa fila sob demanda o "repetir tudo" é do contexto e o motor fica em Off
                    RepeatMode::Off if self.ctx.as_ref().is_some_and(|c| c.repeat_all) => RepeatVm::All,
                    RepeatMode::Off => RepeatVm::Off,
                    RepeatMode::All => RepeatVm::All,
                    RepeatMode::One => RepeatVm::One,
                };
                self.view.set_playback(&self.pb);
            }
            PlaybackEvent::ShuffleChanged { enabled } => {
                self.pb.shuffle = enabled;
                self.view.set_playback(&self.pb);
            }
            PlaybackEvent::OutputChanged { info } => {
                self.apply_output(&info);
                self.view.set_playback(&self.pb);
            }
            PlaybackEvent::SleepTimerElapsed => {
                self.cancel_radio_resume();
                self.view.sleep_timer_elapsed();
            }
            PlaybackEvent::QueueFinished => {
                self.pb.playing = false;
                self.view.set_playback(&self.pb);
                self.on_queue_finished();
            }
            PlaybackEvent::Error { id, message } => {
                let title = id.and_then(|id| self.queue_items.iter().find(|i| i.id == id)).map(|i| i.track.title.clone());
                let is_current = id.is_some() && id == self.current.and_then(|c| self.queue_items.get(c)).map(|i| i.id);
                let reason = friendly_error(&message);
                let text = match &title {
                    Some(t) => format!("Não foi possível tocar “{t}”. {reason}"),
                    None => format!("Erro de reprodução. {reason}"),
                };
                log::warn!("reprodução: {message}");
                if is_current {
                    self.pb.error = text.clone();
                    self.view.set_playback(&self.pb);
                }
                self.toast(&text, true);
            }
        }
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) fn android_restore_state(&mut self, state: PlaybackState) {
        let status = state.status;
        self.on_engine(PlaybackEvent::QueueChanged {
            items: state.queue,
            current: state.current,
        });
        self.pb.playing = status == PlaybackStatus::Playing;
        self.pb.loading = status == PlaybackStatus::Buffering;
        self.pb.shuffle = state.shuffle;
        self.pb.repeat = match state.repeat {
            RepeatMode::Off => RepeatVm::Off,
            RepeatMode::One => RepeatVm::One,
            RepeatMode::All => RepeatVm::All,
        };
        self.pb.volume = sane_unit(state.volume, self.pb.volume);
        self.cfg.volume = self.pb.volume;
        self.position = state.position;
        self.duration = state.duration;
        self.listened = state.position;
        self.listen_sample = None;
        self.listen_started = None;
        self.recorded_current = false;
        self.stalled = false;
        if status == PlaybackStatus::Playing {
            crate::android::request_playback_focus();
            self.pb.error.clear();
        }
        if let Some(item) = self
            .current
            .and_then(|index| self.queue_items.get(index))
            .cloned()
        {
            self.on_track_changed(&item);
        } else {
            self.pb.now = None;
            self.pb.has_now = false;
            self.pb.quality_label.clear();
            self.pb.source.clear();
            self.now_art = NowArtVm::default();
            self.view.set_now_art(&self.now_art);
            self.push_accent();
        }
        if let Some(info) = state.output.as_ref() {
            self.apply_output(info);
        }
        self.view.set_playback(&self.pb);
        self.view.set_volume(self.pb.volume);
        self.push_progress();
    }

    /// "Original" só vale para arquivo que chega sem transformação; o stream do YouTube já vem comprimido (Opus/AAC),
    /// então o que garantimos ali é só "sem equalizador".
    fn untouched_note(remote: bool) -> &'static str {
        if remote { "Qualidade do YouTube Music, sem equalizador" } else { "Original, sem equalizador" }
    }

    fn on_track_changed(&mut self, item: &QueueItem) {
        let vm = self.vm_from_item(item);
        self.pb.has_now = true;
        self.pb.source = ytm_vm::source_of_track(&item.track).to_owned();
        self.pb.quality_label = self.quality_labels.get(&item.id).cloned().or_else(|| item.track.format.as_ref().map(format::quality_label)).unwrap_or_default();
        if let TrackSource::Remote { .. } = &item.track.source {
            self.remember_remote(&item.track);
            if let Some(c) = self.ctx.as_mut() {
                c.played.insert(vm.id.clone());
            }
        }
        // a nota de qualidade acompanha a origem da faixa (o motor só reenvia a saída quando o formato muda)
        let remote = matches!(item.track.source, TrackSource::Remote { .. });
        if self.pb.output_note == Self::untouched_note(!remote) {
            self.pb.output_note = Self::untouched_note(remote).to_owned();
        }
        self.pb.now = Some(vm);
        self.art_seq += 1;
        #[cfg(any(target_os = "android", target_os = "ios"))]
        if let TrackSource::Local { path } = &item.track.source {
            self.jobs.submit(Job::Lyrics { seq: self.art_seq, path: path.clone() });
        }
        match &item.track.cover {
            Some(CoverRef::File { path }) => self.jobs.submit(Job::NowArt { seq: self.art_seq, cover: Some(path.clone()) }),
            Some(CoverRef::Url { url }) => self.jobs.submit(Job::Remote(crate::remote::RemoteJob::NowArt { seq: self.art_seq, url: url.clone() })),
            _ => self.jobs.submit(Job::NowArt { seq: self.art_seq, cover: None }),
        }
    }

    fn apply_output(&mut self, info: &OutputInfo) {
        let default_name = if cfg!(any(target_os = "android", target_os = "ios")) { "Este celular" } else { "Padrão do sistema" };
        self.pb.output_name = match info.device.as_str() {
            d if d.is_empty() || d.to_ascii_lowercase().starts_with("default") => default_name.to_owned(),
            "null" => "Saída simulada".to_owned(),
            other => other.to_owned(),
        };
        self.pb.bit_perfect = info.bit_perfect;
        let remote_now = self.pb.now.as_ref().is_some_and(|t| ytm_vm::is_remote_id(&t.id));
        self.pb.output_note = if info.bit_perfect {
            Self::untouched_note(remote_now).to_owned()
        } else if let Some(from) = info.resampled_from {
            format!("Reamostrado de {} para {} kHz", format::khz(from), format::khz(info.sample_rate))
        } else {
            "Volume ajustado: o sinal não está bit-perfect".to_owned()
        };
        // o selo do player vem do que o motor realmente está tocando
        // faixa remota: o provedor já informou o formato exato (codec, kbps); o decodificador só conhece a taxa
        let provider_label = self.current.and_then(|c| self.queue_items.get(c)).is_some_and(|i| self.quality_labels.contains_key(&i.id));
        if let (Some(src), Some(now), false) = (&info.source, self.pb.now.as_mut(), provider_label) {
            let b = format::badge(&src.codec.to_string(), src.codec.is_lossless(), src.bit_depth, src.sample_rate, src.bitrate_kbps);
            if !b.codec.is_empty() {
                // o motor chama WAV/AIFF de "PCM"; o nome do arquivo (WAV) diz mais ao usuário
                if !(b.codec == "PCM" && !now.codec.is_empty()) {
                    now.codec = b.codec;
                }
                now.detail = b.detail;
                now.detail_long = b.detail_long;
                now.hires = b.hires;
            }
            self.pb.quality_label = format::quality_label(src);
        }
    }

    fn vm_from_item(&self, item: &QueueItem) -> TrackVm {
        let t = &item.track;
        if let TrackSource::Remote { .. } = &t.source {
            let mut vm = ytm_vm::track_vm(t, false, "");
            vm.liked = self.liked.contains(&vm.id);
            vm.playing = true;
            vm.cover = None;
            return vm;
        }
        let id = t.id.to_string();
        let path = match &t.source {
            TrackSource::Local { path } => Some(path.as_path()),
            _ => None,
        };
        let b = match &t.format {
            Some(f) => {
                let codec = if f.codec == kazzu_core::Codec::Pcm { format::pcm_label(path).to_owned() } else { f.codec.to_string() };
                format::badge(&codec, f.codec.is_lossless(), f.bit_depth, f.sample_rate, f.bitrate_kbps)
            }
            None => format::Badge::default(),
        };
        TrackVm {
            liked: self.liked.contains(&id),
            id,
            title: t.title.clone(),
            artist: t.artist_line(),
            album: t.album.as_ref().map(|a| a.title.clone()).unwrap_or_default(),
            duration_label: t.duration.map(format::duration_label).unwrap_or_default(),
            codec: b.codec,
            detail: b.detail,
            detail_long: b.detail_long,
            hires: b.hires,
            playing: true,
            ..Default::default()
        }
    }

    fn upcoming(&self) -> &[QueueItem] {
        match self.current {
            Some(c) if c < self.queue_items.len() => &self.queue_items[c + 1..],
            _ => &[],
        }
    }

    fn publish_queue(&mut self) {
        let up = self.upcoming();
        #[cfg(any(target_os = "android", target_os = "ios"))]
        let queue_ids = up.iter().take(QUEUE_VIEW_MAX).map(|item| item.id).collect::<Vec<_>>();
        let mut tracks: Vec<TrackVm> = up
            .iter()
            .take(QUEUE_VIEW_MAX)
            .map(|i| {
                let id = match &i.track.source {
                    TrackSource::Remote { .. } => ytm_vm::remote_id(i.track.id.as_str()),
                    TrackSource::Local { .. } => i.track.id.to_string(),
                };
                let mut vm = self.vm_cache.peek(&id).cloned().unwrap_or_else(|| {
                    let mut vm = self.vm_from_item(i);
                    vm.cover = None;
                    vm
                });
                vm.playing = false;
                vm.liked = self.liked.contains(&vm.id);
                vm
            })
            .collect();
        let mut total: Duration = up.iter().filter_map(|i| i.track.duration).sum();
        let mut count = up.len();
        // fila gerenciada: o que ainda não foi entregue ao motor também aparece
        if let Some(ctx) = &self.ctx {
            for entry in ctx.tail.iter() {
                count += 1;
                if let Entry::Remote(t) = entry {
                    total += t.duration.unwrap_or_default();
                }
                if tracks.len() < QUEUE_VIEW_MAX {
                    tracks.push(self.entry_vm(entry));
                }
            }
        }
        let hidden = count.saturating_sub(QUEUE_VIEW_MAX);
        let mut vm = QueueVm {
            tracks,
            summary: if count == 0 { String::new() } else { format::queue_summary(count, total) },
            more_label: if hidden == 0 { String::new() } else { format!("e mais {}", format::songs_label(hidden as u64)) },
            note: if (self.ctx.is_some() || self.engine_radio.is_some()) && self.cfg.autoplay_radio && self.remote_enabled { NOTE_AUTOPLAY.to_owned() } else { String::new() },
        };
        self.cover_pass(&mut vm);
        self.queue_vm = vm.clone();
        self.view.set_queue(&vm);
        #[cfg(any(target_os = "android", target_os = "ios"))]
        self.view.set_queue_ids(&queue_ids);
    }

    fn entry_vm(&self, e: &Entry) -> TrackVm {
        match e {
            Entry::Remote(t) => {
                let mut vm = ytm_vm::track_vm(t, false, "");
                vm.liked = self.liked.contains(&vm.id);
                vm
            }
            Entry::Local(id) => self.vm_cache.peek(id).cloned().unwrap_or(TrackVm { id: id.clone(), title: "Carregando".into(), ..Default::default() }),
        }
    }

    fn push_progress(&self) {
        let progress = match self.duration {
            Some(d) if !d.is_zero() => (self.position.as_secs_f32() / d.as_secs_f32()).clamp(0.0, 1.0),
            _ => 0.0,
        };
        self.view.set_seek_available(self.duration.is_some_and(|d| !d.is_zero()));
        #[cfg(any(target_os = "android", target_os = "ios"))]
        self.view.set_position_millis(
            self.position.as_millis().min(u64::MAX as u128) as u64,
            self.duration.map(|d| d.as_millis().min(u64::MAX as u128) as u64),
        );
        self.view.set_progress(progress, &format::duration_label(self.position), &format::remaining_label(self.position, self.duration));
    }

    fn record_listen_if_needed(&mut self) {
        if self.recorded_current {
            return;
        }
        let Some(now) = &self.pb.now else { return };
        let measured = self.engine_quality.is_some() || cfg!(any(target_os = "android", target_os = "ios"));
        let listened = if measured { self.listened } else { self.position };
        let long_enough = listened >= LISTEN_MIN
            || self.duration.is_some_and(|duration| !duration.is_zero() && listened.saturating_mul(2) >= duration);
        if long_enough {
            let listened_ms = listened.as_millis().min(i64::MAX as u128) as u64;
            if ytm_vm::is_remote_id(&now.id) {
                if let Some(track) = self.remote_tracks.peek(&now.id).cloned() {
                    self.jobs.submit(Job::RecordRemotePlay { track: Box::new(track), listened_ms });
                }
            } else {
                self.jobs.submit(Job::RecordPlay { id: now.id.clone(), listened_ms });
            }
        }
        #[cfg(any(target_os = "android", target_os = "ios"))]
        if let (Some(started), Some(duration)) = (self.listen_started, self.duration)
            && duration > Duration::from_secs(30)
            && self.listened >= (duration / 2).min(Duration::from_secs(240))
            && !now.title.is_empty() && !now.artist.is_empty()
        {
            crate::android::last_fm_scrobble(&now.title, &now.artist, &now.album, started, duration.as_secs() as i64);
        }
        self.recorded_current = true;
    }

    // ----------------------------------------------------------------------------------------
    // Comandos da UI
    // ----------------------------------------------------------------------------------------

    pub fn navigate(&mut self, tab: NavTab) {
        self.save_detail_scroll();
        if self.tab == tab {
            self.visits.remove(&tab);
        }
        self.tab = tab;
        self.view.open_detail(false);
        self.detail_open = false;
        self.detail_seq += 1; // um detalhe pedido antes desta troca de aba não pode abrir depois dela
        if matches!(tab, NavTab::Home | NavTab::Explore | NavTab::Search) && self.remote_enabled && !self.explore_requested {
            self.request_explore();
        }
        if matches!(tab, NavTab::Home | NavTab::Search) && self.stale_screens.remove(&NavTab::Explore) {
            self.publish_explore();
        }
        if self.stale_screens.remove(&tab) {
            match tab {
                NavTab::Home => self.publish_home(),
                NavTab::Explore => self.publish_explore(),
                NavTab::Search => self.publish_search(),
                NavTab::Library | NavTab::Settings => {}
            }
        }
        self.restore_visit();
    }

    fn save_detail_scroll(&mut self) {
        if self.detail_open {
            if let Some(page) = self.visits.get_mut(&self.tab).and_then(|stack| stack.last_mut()) {
                page.scroll = self.view.detail_scroll();
            }
        }
    }

    fn visit(&mut self, route: DetailRoute) {
        self.save_detail_scroll();
        let pages = self.visits.entry(self.tab).or_default();
        if pages.last().is_none_or(|page| page.route != route) {
            pages.push(PageVisit { route: route.clone(), scroll: 0.0, focus: String::new() });
        }
        self.load_route(route);
    }

    fn restore_visit(&mut self) {
        if let Some(page) = self.visits.get(&self.tab).and_then(|stack| stack.last()).cloned() {
            self.load_route(page.route);
        } else {
            self.view.restore_focus(self.section_focus.get(&self.tab).map_or("", String::as_str));
        }
    }

    fn restore_detail_scroll(&self) {
        if let Some(page) = self.visits.get(&self.tab).and_then(|stack| stack.last()) {
            self.view.restore_detail_scroll(page.scroll);
            self.view.restore_focus(&page.focus);
        }
    }

    pub fn remember_focus(&mut self, key: &str) {
        if self.detail_open {
            if let Some(page) = self.visits.get_mut(&self.tab).and_then(|pages| pages.last_mut()) {
                page.focus = key.to_owned();
            }
        } else {
            self.section_focus.insert(self.tab, key.to_owned());
        }
    }

    fn load_route(&mut self, route: DetailRoute) {
        self.view.restore_focus("");
        match route {
            DetailRoute::Catalog(what) => self.load_remote_page(what),
            DetailRoute::Library(what) => {
                self.detail_seq += 1;
                self.detail_what = None;
                self.detail_ids.clear();
                self.detail_album = None;
                if self.detail_open || cfg!(any(target_os = "android", target_os = "ios")) {
                    self.show_detail(DetailVm { state: LoadStateVm::Loading, ..Default::default() });
                }
                #[cfg(any(target_os = "android", target_os = "ios"))]
                if what == DetailRef::Downloads && let Some(cache) = self.engine.cache().cloned() {
                    self.jobs.submit(Job::OpenCachedAudio { seq: self.detail_seq, cache });
                    return;
                }
                self.jobs.submit(Job::OpenDetail { seq: self.detail_seq, what });
            }
        }
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) fn android_close_detail(&mut self) {
        self.view.open_detail(false);
        self.detail_closed();
    }

    pub fn detail_closed(&mut self) {
        self.detail_open = false;
        self.detail_seq += 1;
        if let Some(pages) = self.visits.get_mut(&self.tab) {
            pages.pop();
        }
        self.restore_visit();
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) fn android_room_playlist_track(&mut self, playlist: String, id: String, title: String, artist: String) {
        if !self.library.playlists.iter().any(|item| item.id == playlist)
            || id.is_empty() || id.len() > 32
            || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return;
        }
        let mut track = CoreTrack::new(id.clone(), title, TrackSource::Remote {
            provider: kazzu_core::ProviderKind::YtMusic, id: id.clone(),
        });
        if !artist.is_empty() {
            track.artists.push(kazzu_core::ArtistRef::named(artist));
        }
        self.jobs.submit(Job::AddToPlaylist {
            playlist_id: playlist, track_id: ytm_vm::remote_id(&id), remote: Some(Box::new(track)),
        });
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) fn android_set_following(&mut self, following: bool) {
        self.following_room = following;
        self.followed_id = None;
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) fn android_wait_for_room(&mut self) {
        if self.following_room && self.followed_id.is_some() {
            self.engine.pause();
        }
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) fn android_follow_track(&mut self, id: String, title: String, artist: String, position: i64, playing: bool) -> bool {
        if !self.following_room || self.engine_quality.is_none() || id.is_empty() || id.len() > 32
            || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return false;
        }
        let position = Duration::from_millis(position.clamp(0, 86_400_000) as u64);
        let now_matches = self.pb.now.as_ref().is_some_and(|track| ytm_vm::strip_remote(&track.id) == Some(id.as_str()));
        if !now_matches {
            if Some(&id) == self.followed_id.as_ref() && self.pb.loading {
                return true;
            }
            if playing && !self.android_has_focus() {
                return false;
            }
            let mut track = CoreTrack::new(id.clone(), title, TrackSource::Remote {
                provider: kazzu_core::ProviderKind::YtMusic, id: id.clone(),
            });
            if !artist.is_empty() {
                track.artists.push(kazzu_core::ArtistRef::named(artist));
            }
            self.ctx_seq += 1;
            self.ctx = None;
            self.engine_radio = None;
            self.followed_id = Some(id);
            self.engine.load_at(QueueItem::new(track, kazzu_core::StreamSource::Deferred), position, playing);
        } else {
            self.followed_id = Some(id);
            if playing && !self.pb.playing && !self.pb.loading && self.android_has_focus() {
                self.engine.play();
            } else if !playing && (self.pb.playing || self.pb.loading) {
                self.engine.pause();
            }
            if !self.pb.loading && self.position.abs_diff(position) > Duration::from_millis(1300) {
                self.listen_sample = None;
                self.engine.seek(position);
            }
        }
        true
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) fn android_play_track(&mut self, id: &str) {
        if self.android_has_focus() {
            self.play_track(id);
        }
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) fn android_pause_for_system(&mut self) {
        self.cancel_radio_resume();
        self.engine.pause();
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) fn android_toggle_play(&mut self) {
        if self.pb.playing || self.pb.loading || self.android_has_focus() {
            self.toggle_play();
        }
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) fn android_media_seek(&mut self, position_ms: i64) {
        self.listen_sample = None;
        if let Some(duration) = self.duration {
            let position = Duration::from_millis(position_ms.max(0) as u64).min(duration);
            self.engine.seek(position);
        }
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    fn android_has_focus(&mut self) -> bool {
        if crate::android::request_playback_focus() {
            true
        } else {
            self.notice("A saída de áudio está ocupada.", true);
            false
        }
    }

    pub fn toggle_play(&mut self) {
        if self.pb.has_now {
            self.engine.toggle_pause();
        } else if let Some(first) = self.recents_ids.first().cloned() {
            self.play_context(&first);
        } else {
            self.toast("Adicione uma pasta de músicas para começar a ouvir.", false);
        }
    }

    pub fn next(&mut self) {
        self.engine.next();
    }

    pub fn previous(&mut self) {
        self.engine.previous();
    }

    /// `progress` 0..1.
    pub fn seek(&mut self, progress: f32) {
        self.listen_sample = None;
        if let Some(d) = self.duration {
            let target = d.mul_f32(sane_unit(progress, 0.0));
            self.engine.seek(target);
            self.position = target;
            self.push_progress();
        }
    }

    pub fn seek_relative(&mut self, secs: i64) {
        self.listen_sample = None;
        let Some(d) = self.duration else { return };
        let step = secs.signum() * SEEK_STEP_SECS.max(secs.abs());
        let target = if step >= 0 { self.position + Duration::from_secs(step as u64) } else { self.position.saturating_sub(Duration::from_secs(step.unsigned_abs())) };
        let target = target.min(d);
        self.engine.seek(target);
        self.position = target;
        self.push_progress();
    }

    pub fn set_volume(&mut self, v: f32) {
        let v = sane_unit(v, self.pb.volume);
        self.pb.volume = v;
        self.cfg.volume = (v * 1000.0).round() / 1000.0;
        self.cfg_dirty = true;
        self.engine.set_volume(v);
    }

    pub fn volume_step(&mut self, dir: i32) {
        let v = ((self.pb.volume / VOLUME_STEP).round() + dir as f32) * VOLUME_STEP;
        self.set_volume(v.clamp(0.0, 1.0));
        self.view.set_volume(self.pb.volume);
    }

    pub fn retry_track(&mut self) {
        if self.stalled {
            // a rede caiu no meio da faixa: o motor retoma de onde parou
            self.stalled = false;
            self.pb.error.clear();
            self.view.set_playback(&self.pb);
            self.engine.play();
        } else if let Some(c) = self.current {
            self.pb.error.clear();
            self.view.set_playback(&self.pb);
            self.engine.load(self.queue_items.clone(), c);
        }
    }

    /// Clique numa faixa (linha, destaque ou resultado): a lista de origem vira a fila.
    pub fn play_track(&mut self, id: &str) {
        self.play_context(id);
    }

    fn remember_search(&mut self) {
        if self.search_query.trim().len() >= 2 {
            self.cfg.push_recent_search(&self.search_query);
            self.save_config();
            self.publish_search();
        }
    }

    pub fn open_collection(&mut self, id: &str) {
        match id {
            "history" => self.request_detail(DetailRef::History),
            "songs" => self.request_detail(DetailRef::Songs),
            _ if id.starts_with("artist:") => self.request_detail(DetailRef::Artist(id.trim_start_matches("artist:").to_owned())),
            "downloads" => self.request_detail(DetailRef::Downloads),
            "liked" => self.request_detail(DetailRef::Liked),
            "albums" => self.request_detail(DetailRef::Albums),
            _ => {
                let msg = if self.library.playlists.is_empty() { "Você ainda não criou playlists. Use Criar playlist." } else { "Suas playlists estão logo abaixo." };
                self.toast(msg, false);
            }
        }
    }

    fn request_detail(&mut self, what: DetailRef) {
        self.visit(DetailRoute::Library(what));
    }

    pub fn open_genre(&mut self, id: &str) {
        self.request_detail(DetailRef::Genre(id.to_owned()));
    }

    pub fn toggle_like(&mut self, id: &str) {
        if id.is_empty() {
            return;
        }
        let liked = !self.liked.contains(id);
        if liked { self.liked.insert(id.to_owned()); } else { self.liked.remove(id); }
        self.pending_likes.insert(id.to_owned(), liked);
        if let Some(now) = self.pb.now.as_mut().filter(|t| t.id == id) {
            now.liked = liked;
            self.view.set_playback(&self.pb);
        }
        self.view.mark_liked(id, liked);
        self.refresh_liked_count();
        if ytm_vm::is_remote_id(id) {
            match self.remote_tracks.peek(&id.to_owned()).cloned() {
                Some(track) => self.jobs.submit(Job::Remote(crate::remote::RemoteJob::Like { track, liked })),
                None => {
                    // sem os dados da faixa não dá para guardar; desfaz a marca
                    self.liked.remove(id);
                    self.pending_likes.remove(id);
                    self.view.mark_liked(id, false);
                    self.toast("Não consegui guardar esta curtida. Abra a música de novo e tente outra vez.", true);
                    return;
                }
            }
        } else {
            self.jobs.submit(Job::SetLiked { id: id.to_owned(), liked });
        }
        let text = if liked { "Adicionada às Curtidas" } else { "Removida das Curtidas" };
        self.toast_with(text, false, "Desfazer", ToastAction::UndoLike { id: id.to_owned(), was_liked: !liked });
    }

    fn refresh_liked_count(&mut self) {
        if let Some(c) = self.library.collections.iter_mut().find(|c| c.id == "liked") {
            c.count_label = format::songs_label(self.liked.len() as u64);
            self.view.set_library(&self.library);
        }
    }

    pub fn create_playlist(&mut self, name: &str, with_track: &str) {
        let add = if with_track.is_empty() {
            None
        } else {
            let remote = ytm_vm::is_remote_id(with_track).then(|| self.remote_tracks.peek(&with_track.to_owned()).cloned()).flatten();
            Some((with_track.to_owned(), remote.map(Box::new)))
        };
        self.jobs.submit(Job::CreatePlaylist { name: name.to_owned(), add });
    }

    pub fn add_to_playlist(&mut self, track_id: &str, playlist_id: &str) {
        let remote = ytm_vm::is_remote_id(track_id).then(|| self.remote_tracks.peek(&track_id.to_owned()).cloned()).flatten();
        if ytm_vm::is_remote_id(track_id) && remote.is_none() {
            self.toast("Não consegui adicionar esta música. Abra-a de novo e tente outra vez.", true);
            return;
        }
        self.jobs.submit(Job::AddToPlaylist { playlist_id: playlist_id.to_owned(), track_id: track_id.to_owned(), remote: remote.map(Box::new) });
    }

    pub fn queue_track(&mut self, id: &str, mode: QueueMode) {
        if ytm_vm::is_remote_id(id) {
            if let Some(track) = self.remote_tracks.peek(&id.to_owned()).cloned() {
                if self.engine_quality.is_some() {
                    self.queued(vec![QueueItem::new(track, kazzu_core::StreamSource::Deferred)], mode == QueueMode::PlayNext);
                } else {
                    self.jobs.submit(Job::Remote(crate::remote::RemoteJob::ResolveQueued { track, quality: self.cfg.stream_quality.to_core(), play_next: mode == QueueMode::PlayNext }));
                }
            }
        } else {
            self.jobs.submit(Job::QueueItems { id: id.to_owned(), mode });
        }
    }

    /// Entrega itens prontos ao motor ("Tocar em seguida" / "Adicionar à fila") e oferece Desfazer.
    fn queued(&mut self, items: Vec<QueueItem>, play_next: bool) {
        let n = items.len();
        let ids: Vec<QueueItemId> = items.iter().map(|i| i.id).collect();
        if play_next { self.engine.play_next(items) } else { self.engine.enqueue(items) }
        let text = match (play_next, n) {
            (true, 1) => "Toca em seguida",
            (true, _) => "Tocam em seguida",
            (false, 1) => "Adicionada à fila",
            (false, _) => "Adicionadas à fila",
        };
        self.toast_with(text, false, "Desfazer", ToastAction::UndoQueue(ids));
    }

    pub fn queue_clear(&mut self) {
        self.ctx_seq += 1;
        self.engine_radio = None;
        if let Some(ctx) = self.ctx.as_mut() {
            ctx.tail.clear();
            ctx.radio_exhausted = true;
        }
        self.engine.clear();
    }

    pub fn clear_recent_searches(&mut self) {
        self.cfg.recent_searches.clear();
        self.save_config();
        self.publish_search();
    }

    pub fn remove_recent_search(&mut self, text: &str) {
        self.cfg.recent_searches.retain(|s| s != text);
        self.save_config();
        self.publish_search();
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub fn add_folder(&mut self) {
        if !crate::android::select_music_folder() {
            self.notice("Seletor de pastas indisponível.", true);
        }
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) fn android_register_imports(&mut self) {
        if let Some(root) = self.data_dir.as_ref().map(|dir| dir.join("imported-music")).filter(|dir| dir.is_dir()) {
            self.add_folder_path(root);
        }
    }

    pub fn browse_folders(&mut self, _path: &str) {
        #[cfg(any(target_os = "android", target_os = "ios"))]
        self.add_folder();
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        self.add_folder();
    }

    pub fn add_named_folder(&mut self, folder: &str) {
        let path = PathBuf::from(folder.trim());
        if !path.is_absolute() {
            self.toast("Informe o caminho completo de uma pasta.", true);
            return;
        }
        #[cfg(any(target_os = "android", target_os = "ios"))]
        self.add_folder();
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        self.add_folder_path(path);
    }

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    pub fn add_folder(&mut self) {
        let sink = self.sink.clone();
        let spawned = std::thread::Builder::new().name("kazzu-folder-dialog".into()).spawn(move || {
            let picked = pollster::block_on(rfd::AsyncFileDialog::new().set_title("Escolha a pasta de músicas").pick_folder());
            sink(Msg::FolderPicked(picked.map(|h| h.path().to_path_buf())));
        });
        if spawned.is_err() {
            self.toast("Não foi possível abrir o seletor de pastas.", true);
        }
    }

    #[cfg(target_os = "ios")]
    pub fn add_folder(&mut self) {
        self.notice("Selecione as pastas na tela de configuração do iOS.", false);
    }

    /// Adiciona uma pasta já conhecida (ex.: `--demo-library`).
    pub fn add_folder_path(&mut self, path: PathBuf) {
        self.jobs.submit(Job::AddFolder(path));
    }

    pub fn remove_folder(&mut self, path: &str) {
        self.jobs.submit(Job::RemoveFolder(PathBuf::from(path)));
    }

    pub fn rescan(&mut self) {
        if !self.scanning {
            self.jobs.submit(Job::Scan);
        }
    }

    pub fn coming_soon(&mut self, what: &str) {
        self.toast(&format!("{what} chega em uma próxima versão."), false);
    }

    pub fn clear_history(&mut self) {
        self.jobs.submit(Job::ClearHistory);
    }

    /// "Relatar um problema": grava um texto local com versão e sistema. Nada é enviado.
    pub fn report_problem(&mut self) {
        let Some(dir) = self.data_dir.clone() else {
            self.toast("Não achei onde guardar o relato.", true);
            return;
        };
        let text = format!(
            "Relato do Kazzu\nVersão: {}\nSistema: {} {}\n\nEscreva aqui o que aconteceu e envie este arquivo por e-mail ou como issue no GitHub. O Kazzu não envia nada sozinho.\n",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH
        );
        self.jobs.submit(Job::WriteReport { path: dir.join("kazzu-relato.txt"), text });
    }

    // ----------------------------------------------------------------------------------------
    // Toast com ação, desempenho e preferências
    // ----------------------------------------------------------------------------------------

    /// Botão do aviso ("Desfazer", "Tentar de novo").
    pub fn toast_action(&mut self) {
        match std::mem::replace(&mut self.toast_action, ToastAction::None) {
            ToastAction::None => {}
            ToastAction::UndoLike { id, was_liked } => {
                // volta ao estado anterior: toggle_like inverte o atual
                if self.liked.contains(&id) != was_liked {
                    self.toggle_like(&id);
                    self.toast_action = ToastAction::None;
                }
            }
            ToastAction::UndoQueue(ids) => {
                for id in ids {
                    self.engine.remove(id);
                }
            }
            ToastAction::RetryPlay => self.retry_track(),
            ToastAction::RetryStart => self.retry_start(),
            ToastAction::RetryLookahead => self.unstall(true),
            ToastAction::RetryReresolve(id) => {
                self.expired_log.remove(&id);
                self.pb.error.clear();
                self.view.set_playback(&self.pb);
                self.reresolve(id);
            }
        }
    }

    pub fn perf_offer_accept(&mut self) {
        self.perf_dismissed = true;
        self.view.set_perf_offer(false);
        self.set_reduce_transparency(true);
    }

    pub fn perf_offer_dismiss(&mut self) {
        self.perf_dismissed = true;
        self.view.set_perf_offer(false);
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) fn android_set_playlist_cover(&mut self, id: String, bytes: Vec<u8>) {
        self.jobs.submit(Job::SetPlaylistCover { id, bytes });
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    fn android_choose_playlist_cover(&mut self) {
        let id = &self.detail_vm.editable_playlist;
        if !id.is_empty() && !crate::android::select_playlist_cover(id) {
            self.notice("Seletor de imagens indisponível.", true);
        }
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) fn android_restore_backup(&mut self, json: String) {
        let Some(root) = self.data_dir.as_ref().map(|dir| dir.join("imported-music")) else { return };
        self.jobs.submit(Job::RestoreBackup { root, json });
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) fn android_sync_peer_playlists(&mut self, json: String) {
        let Some(root) = self.data_dir.as_ref().map(|dir| dir.join("imported-music")) else { return };
        self.jobs.submit(Job::SyncPeerPlaylists { root, json });
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) fn android_host_room(&mut self, enabled: bool, share: bool, playlist: &str) {
        crate::android::host_remote(enabled, share, playlist);
        if enabled && share {
            self.android_publish_room_backup();
        }
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    fn android_create_backup(&mut self, lan: bool) {
        let Some(root) = self.data_dir.as_ref().map(|dir| dir.join("imported-music")) else { return };
        self.jobs.submit(Job::ExportBackup { root, config: Box::new(self.cfg.clone()), lan, room: false });
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    fn android_publish_room_backup(&mut self) {
        let Some(root) = self.data_dir.as_ref().map(|dir| dir.join("imported-music")) else { return };
        self.jobs.submit(Job::ExportBackup { root, config: Box::new(self.cfg.clone()), lan: true, room: true });
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) fn android_import_m3u(&mut self, title: String, contents: String) {
        self.jobs.submit(Job::ImportM3u { title, contents });
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) fn android_edit_tags(&mut self, id: &str, title: &str, artist: &str, album: &str, genre: &str) {
        let Some(root) = self.data_dir.as_ref().map(|path| path.join("imported-music")) else { return };
        if title.trim().is_empty() || [title, artist, album, genre].iter().any(|value| value.len() > 4096) {
            self.notice("Informe um título e confira o tamanho dos campos.", true);
            return;
        }
        let field = |value: &str| value.trim().to_owned();
        self.jobs.submit(Job::EditLocalTags {
            root, track_id: id.to_owned(), title: field(title), artist: field(artist),
            album: field(album), genre: field(genre),
        });
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) fn android_stats(&mut self, period: i32, export: bool) {
        let Some(from) = crate::android::listening_period_start(period) else {
            self.notice("Não foi possível calcular o período de escuta.", true);
            return;
        };
        let until = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
            .map_or(from, |elapsed| (elapsed.as_secs() as i64).saturating_add(1));
        if !export {
            self.stats_seq += 1;
        }
        self.jobs.submit(Job::ListeningStats { seq: self.stats_seq, from, until, export });
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    fn android_export_stats(&mut self, stats: &kazzu_library::ListeningStats) {
        let ranks = |values: &[kazzu_library::ListeningRank]| values.iter().map(|rank| {
            serde_json::json!({
                "title": rank.title, "subtitle": rank.subtitle, "plays": rank.plays, "listened_ms": rank.listened_ms
            })
        }).collect::<Vec<_>>();
        let json = serde_json::json!({
            "from_unix": stats.from, "until_unix": stats.until,
            "listened_ms": stats.listened_ms, "plays": stats.plays,
            "songs": ranks(&stats.songs), "artists": ranks(&stats.artists),
            "albums": ranks(&stats.albums), "genres": ranks(&stats.genres)
        });
        match serde_json::to_string_pretty(&json) {
            Ok(export) if crate::android::export_listening_stats(&export) => {}
            _ => self.notice("Não foi possível exportar as estatísticas.", true),
        }
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) fn android_create_playlist(&mut self, name: &str) {
        self.create_playlist(name, "");
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) fn android_rename_playlist(&mut self, name: &str) {
        let name = name.trim();
        if !name.is_empty() {
            self.edit_playlist(crate::worker::PlaylistEdit::Rename(name.to_owned()));
        }
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) fn android_delete_playlist(&mut self) {
        self.edit_playlist(crate::worker::PlaylistEdit::Delete);
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) fn android_edit_playlist_row(&mut self, action: &str, index: i32, track: &str) {
        let Ok(index) = usize::try_from(index) else { return };
        let track = track.to_owned();
        let edit = match action {
            "remove" => crate::worker::PlaylistEdit::Remove { index, track },
            "move" => crate::worker::PlaylistEdit::MoveUp { index, track },
            _ => return,
        };
        self.edit_playlist(edit);
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) fn android_add_now_to_playlist(&mut self, playlist: &str) {
        if let Some(now) = self.pb.now.as_ref() {
            self.add_to_playlist(&now.id.clone(), playlist);
        }
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) fn android_queue_track(&mut self, id: &str, next: bool) {
        self.queue_track(id, if next { QueueMode::PlayNext } else { QueueMode::Enqueue });
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) fn android_lyrics(&mut self) {
        let Some(id) = self.pb.now.as_ref().and_then(|track| ytm_vm::strip_remote(&track.id)) else { return };
        if self.remote_enabled {
            self.view.set_remote_lyrics("", "", true);
            self.jobs.submit(Job::Remote(crate::remote::RemoteJob::Lyrics { seq: self.art_seq, id: id.to_owned() }));
        }
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) fn android_watch_now(&mut self) {
        let Some(id) = self.pb.now.as_ref().and_then(|track| ytm_vm::strip_remote(&track.id)) else { return };
        if id.is_empty() || id.len() > 32 || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
            return;
        }
        if crate::android::open_video(id) {
            self.engine.pause();
        } else {
            self.notice("Não foi possível abrir o reprodutor de vídeo.", true);
        }
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) fn android_share_now(&mut self) {
        let Some(now) = self.pb.now.as_ref() else { return };
        let mut text = format!("{} — {}", now.title, now.artist);
        if let Some(id) = ytm_vm::strip_remote(&now.id).filter(|id| {
            !id.is_empty() && id.len() <= 32 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        }) {
            text.push_str(&format!("\nhttps://music.youtube.com/watch?v={id}"));
        }
        if !crate::android::share_text(&text) {
            self.notice("Não foi possível abrir o compartilhamento.", true);
        }
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) fn android_home_feed(&mut self, feed: i32) {
        let section = match feed {
            1 => HomeSectionVm::Mixes,
            2 => HomeSectionVm::Discover,
            _ => HomeSectionVm::ForYou,
        };
        if self.remote_enabled {
            self.request_home(section);
        }
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) fn android_open_item(&mut self, kind: &str, id: &str) {
        if kind == "favorite-mix" {
            self.request_detail(DetailRef::FavoriteMix);
        } else if kind == "smart-recent" {
            self.request_detail(DetailRef::SmartRecent);
        } else if kind == "smart-unplayed" {
            self.request_detail(DetailRef::SmartUnplayed);
        } else if kind == "genre" {
            self.open_genre(id);
        } else if matches!(kind, "history" | "songs" | "downloads" | "albums") {
            self.open_collection(kind);
        } else if matches!(kind, "album" | "playlist" | "artist" | "mood" | "liked") {
            self.open_item(kind, id);
        } else {
            self.android_play_track(id);
        }
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) fn android_play_detail(&mut self) {
        if self.android_has_focus() {
            self.play_all(false);
        }
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) fn android_navigation(&mut self, tab: i32) {
        let tab = match tab {
            0 => NavTab::Home,
            1 => NavTab::Explore,
            2 => NavTab::Library,
            3 => NavTab::Search,
            _ => NavTab::Settings,
        };
        if tab != self.tab {
            self.navigate(tab);
        }
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) fn android_storage_usage(&mut self) {
        let (Some(files), Some(cache)) = (self.data_dir.clone(), self.cache_dir.clone()) else { return };
        let sink = self.sink.clone();
        let _ = std::thread::Builder::new().name("kazzu-storage".into()).spawn(move || {
            fn bytes(path: &std::path::Path) -> u64 {
                let mut stack = vec![path.to_owned()];
                let mut bytes = 0u64;
                while let Some(dir) = stack.pop() {
                    let Ok(files) = std::fs::read_dir(dir) else { continue };
                    for file in files.flatten() {
                        let Ok(kind) = file.file_type() else { continue };
                        if kind.is_dir() {
                            stack.push(file.path());
                        } else if kind.is_file() && let Ok(metadata) = file.metadata() {
                            bytes = bytes.saturating_add(metadata.len());
                        }
                    }
                }
                bytes
            }
            sink(Msg::StorageUsage { files: bytes(&files), cache: bytes(&cache) });
        });
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) fn android_remove_cached(&mut self, id: &str) {
        let (Some(track), Some(cache)) = (self.remote_tracks.peek(&id.to_owned()).cloned(), self.engine.cache().cloned()) else { return };
        let sink = self.sink.clone();
        let _ = std::thread::Builder::new().name("kazzu-cache-evict".into()).spawn(move || {
            sink(Msg::StreamRemoved(cache.remove_track(&track)));
        });
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    fn android_clear_cache(&mut self) {
        let Some(cache) = self.engine.cache().cloned() else {
            self.notice("Cache de áudio indisponível.", true);
            return;
        };
        let sink = self.sink.clone();
        if std::thread::Builder::new().name("kazzu-cache".into()).spawn(move || {
            sink(Msg::StreamCacheCleared(cache.clear_and_count()));
        }).is_err() {
            self.notice("Não foi possível iniciar a limpeza do cache.", true);
        }
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) fn android_glass(&mut self, index: i32, value: f32) {
        let Ok(index) = usize::try_from(index) else { return };
        let Some(slot) = self.cfg.mobile_appearance.glass.get_mut(index) else { return };
        *slot = value;
        self.cfg.mobile_appearance.sanitize();
        self.view.set_mobile_appearance(&self.cfg.mobile_appearance);
        // The scrubber emits while moving. Flush this with the normal position
        // updates, background transition or shutdown instead of writing each step.
        self.cfg_dirty = true;
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub(crate) fn android_setting(&mut self, key: &str, value: i32) {
        match key {
            "rounded" | "effects" | "vinyl" => {
                let appearance = &mut self.cfg.mobile_appearance;
                match key {
                    "rounded" => appearance.rounded = value != 0,
                    "effects" => appearance.effects = value,
                    _ => appearance.vinyl = value,
                }
                appearance.sanitize();
                self.view.set_mobile_appearance(&self.cfg.mobile_appearance);
                self.save_config();
            }
            "crossfade" => {
                let seconds = value.clamp(0, 8);
                self.cfg.mobile_appearance.crossfade_seconds = seconds;
                self.engine.set_crossfade(Duration::from_secs(seconds as u64));
                self.view.set_mobile_appearance(&self.cfg.mobile_appearance);
                self.save_config();
            }
            "dark" => self.set_theme(if value != 0 { ThemePref::Dark } else { ThemePref::Light }),
            "motion" => self.set_reduce_motion(value == 0),
            "transparency" => self.set_reduce_transparency(value != 0),
            "cover" => self.set_accent_pref(if value != 0 { AccentPref::Cover } else { AccentPref::Brand }),
            "autoplay" => self.set_autoplay_radio(value != 0),
            "quality" => self.set_stream_quality(StreamQualityPref::from_index(value)),
            "playlistCover" => self.android_choose_playlist_cover(),
            "storageUsage" => self.android_storage_usage(),
            "createBackup" => self.android_create_backup(false),
            "shareBackup" => self.android_create_backup(true),
            "restoreBackup" => {
                if !crate::android::select_backup() {
                    self.notice("Seletor de arquivos indisponível.", true);
                }
            }
            "clearCache" => self.android_clear_cache(),
            "folder" => self.add_folder(),
            "importM3u" => {
                if !crate::android::select_m3u() {
                    self.notice("Seletor de arquivos indisponível.", true);
                }
            }
            "findDuplicates" => crate::android::find_duplicates(),
            "scanLibrary" => {
                crate::android::refresh_music_folders();
                self.rescan();
            }
            "clearSearchHistory" => self.clear_recent_searches(),
            _ => self.notice("Opção indisponível.", false),
        }
    }

    pub fn set_color_palette(&mut self, palette: i32) {
        self.cfg.color_palette = palette.clamp(0, 5);
        self.save_config();
    }

    pub fn set_visual_style(&mut self, style: i32, artwork: bool) {
        self.cfg.glass_style = style.clamp(0, 3);
        self.cfg.artwork_background = artwork;
        self.save_config();
    }

    pub fn set_library_layout(&mut self, view: i32, density: i32) {
        self.cfg.library_view = view.clamp(0, 1);
        self.cfg.library_density = density.clamp(0, 2);
        self.save_config();
    }

    pub fn set_theme(&mut self, theme: ThemePref) {
        self.cfg.theme = theme;
        self.push_prefs();
        self.save_config();
    }

    pub fn set_accent_pref(&mut self, accent: AccentPref) {
        self.cfg.accent = accent;
        self.push_prefs();
        self.push_accent();
        self.save_config();
    }

    pub fn set_reduce_transparency(&mut self, on: bool) {
        self.cfg.reduce_transparency = Some(on);
        self.push_prefs();
        self.save_config();
    }

    pub fn set_reduce_motion(&mut self, on: bool) {
        self.cfg.reduce_motion = Some(on);
        self.push_prefs();
        self.save_config();
    }

    pub fn set_stream_quality(&mut self, q: StreamQualityPref) {
        if let Some(quality) = &self.engine_quality {
            quality.store(q.index(), Ordering::Relaxed);
        }
        self.cfg.stream_quality = q;
        self.push_prefs();
        self.save_config();
    }

    pub fn set_autoplay_radio(&mut self, on: bool) {
        self.cfg.autoplay_radio = on;
        self.push_prefs();
        self.save_config();
        self.publish_queue();
        if on {
            self.ensure_lookahead();
        }
    }

    pub fn dismiss_first_run_banner(&mut self) {
        self.cfg.first_run_dismissed = true;
        self.save_config();
        self.publish_home();
    }

    fn push_prefs(&self) {
        let prefs = PrefsVm {
            theme: self.cfg.theme,
            accent: self.cfg.accent,
            reduce_motion: self.cfg.reduce_motion.unwrap_or(false),
            reduce_transparency: self.cfg.reduce_transparency.unwrap_or(false),
            stream_quality: self.cfg.stream_quality,
            autoplay_radio: self.cfg.autoplay_radio,
        };
        self.view.set_prefs(&prefs);
        #[cfg(any(target_os = "android", target_os = "ios"))]
        self.view.set_mobile_appearance(&self.cfg.mobile_appearance);
    }

    fn push_accent(&self) {
        let accent = if self.pb.has_now { self.now_art.accent } else { Accent::BRAND };
        self.view.set_accent(&accent, self.cfg.accent == AccentPref::Cover);
    }

    fn publish_settings(&self) {
        let providers = vec![
            ProviderVm {
                id: "ytmusic".into(),
                name: ytm_vm::SOURCE_NAME.into(),
                status_label: if self.remote_enabled { "Ativo como visitante.".into() } else { "Desligado".into() },
                connected: self.remote_enabled,
                available: self.remote_enabled,
            },
            ProviderVm { id: "local".into(), name: "Arquivos neste aparelho".into(), status_label: self.settings.summary.clone(), connected: self.settings.has_library, available: true },
            ProviderVm { id: "spotify".into(), name: "Spotify".into(), status_label: "Importação de metadados".into(), connected: false, available: true },
            ProviderVm { id: "tidal-qobuz".into(), name: "Tidal e Qobuz".into(), status_label: "Em breve".into(), connected: false, available: false },
        ];
        self.view.set_settings(&self.settings, &providers);
    }

    /// Pede a gravação da configuração ao worker (nenhum I/O na thread de UI).
    fn save_config(&mut self) {
        self.cfg_dirty = false;
        if let Some(path) = &self.cfg_path {
            self.jobs.submit(Job::SaveConfig { path: path.clone(), config: Box::new(self.cfg.clone()) });
        }
    }

    fn flag_tracks(&self, tracks: &mut [TrackVm]) {
        let now = self.pb.now.as_ref().map(|t| t.id.as_str());
        for t in tracks {
            t.playing = Some(t.id.as_str()) == now;
            t.liked = self.liked.contains(&t.id);
        }
    }

    /// Grava a configuração se algo mudou desde a última gravação (usado pelo temporizador do volume).
    pub fn flush_config(&mut self) {
        if self.cfg_dirty {
            self.save_config();
        }
    }

    /// Mostra um aviso do app (ex.: configuração corrompida na abertura).
    pub fn notice(&mut self, text: &str, error: bool) {
        self.toast(text, error);
    }

    /// Fim do app: grava a configuração e para o motor. Seguro chamar mais de uma vez.
    pub fn shutdown(&mut self) {
        // On Android the library worker drains submissions before closing its
        // channel, so writes stay ordered across UI teardown and backgrounding.
        #[cfg(any(target_os = "android", target_os = "ios"))]
        if self.cfg_dirty {
            self.save_config();
        }

        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        if let Some(path) = &self.cfg_path
            && let Err(e) = self.cfg.save(path)
        {
            log::warn!("{e}");
        }
        self.cfg_dirty = false;
        self.record_listen_if_needed();
        self.engine.stop();
    }

    #[cfg(test)]
    pub fn upcoming_ids(&self) -> Vec<String> {
        self.upcoming().iter().map(|i| i.track.id.to_string()).collect()
    }
}
