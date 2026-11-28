//! Android composition root. All Slint and Controller access stays on the event loop.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use kazzu_audio::{CacheConfig, Engine, EngineConfig, EngineHandle};
use slint::ComponentHandle;

use crate::android_paths::AndroidPaths;
use crate::android_view::AndroidView;
use crate::catalog::{Providers, YtmCatalog};
use crate::config::Config;
use crate::controller::{Controller, Msg, MsgSink};
use crate::covers::CoverFetcher;
use crate::event_bridge::{spawn_engine_bridge, spawn_stream_bridge};
use crate::remote::RemoteWorker;
use crate::ui::{AppBridge, AppWindow};
use crate::worker::ThreadedRunner;

type AppController = Rc<RefCell<Controller<AndroidView>>>;

thread_local! {
    static CONTROLLER: RefCell<Option<AppController>> = const { RefCell::new(None) };
}

static ACTIVITY_WINDOW: Mutex<Option<slint::Weak<AppWindow>>> = Mutex::new(None);
static LAST_ACCESSIBILITY: Mutex<Option<String>> = Mutex::new(None);
static ENGINE_SESSION: OnceLock<Mutex<Option<EngineSession>>> = OnceLock::new();
static WINDOW_GENERATION: AtomicU64 = AtomicU64::new(0);

struct EngineSession {
    engine: EngineHandle,
    quality: Arc<AtomicI32>,
    remote: bool,
}

pub(crate) struct AndroidBackend {
    generation: u64,
}

impl AndroidBackend {
    fn session_slot() -> &'static Mutex<Option<EngineSession>> {
        ENGINE_SESSION.get_or_init(|| Mutex::new(None))
    }

    pub(crate) fn shutdown_engine() {
        let session = Self::session_slot()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        if let Some(session) = session {
            session.engine.shutdown();
        }
    }

    pub(crate) fn media_action(action: String, position_ms: i64) {
        let window = ACTIVITY_WINDOW
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let Some(window) = window else { return };
        let _ = window.upgrade_in_event_loop(move |window| {
            let bridge = window.global::<AppBridge>();
            CONTROLLER.with(|slot| {
                let Some(controller) = slot.borrow().clone() else {
                    return;
                };
                let mut controller = controller.borrow_mut();
                match action.as_str() {
                    "toggle" => controller.android_toggle_play(),
                    "play" if !controller.playback().playing => controller.android_toggle_play(),
                    "pause" => controller.android_pause_for_system(),
                    "stop" => controller.engine().stop(),
                    "next" => controller.next(),
                    "previous" => controller.previous(),
                    "seek" => controller.android_media_seek(position_ms),
                    "playlist-import-failed" => {
                        controller.notice("Não foi possível ler o arquivo M3U.", true)
                    }
                    "duplicates-found" => controller.notice(
                        &format!("{position_ms} cópias idênticas encontradas."),
                        false,
                    ),
                    "duplicates-failed" => {
                        controller.notice("Não foi possível verificar duplicatas.", true)
                    }
                    "folder-imported" => controller.android_register_imports(),
                    "cover-read-failed" => {
                        controller.notice("Não foi possível ler a nova capa.", true)
                    }
                    "backup-exported" => controller.notice("Backup exportado.", false),
                    "backup-export-failed" => {
                        controller.notice("Não foi possível gravar o backup.", true)
                    }
                    "peer-backup-failed" => controller
                        .notice("O backup LAN está indisponível ou o código expirou.", true),
                    "backup-read-failed" => {
                        controller.notice("Não foi possível ler o backup.", true)
                    }
                    "stats-exported" => controller.notice("Estatísticas exportadas.", false),
                    "stats-export-failed" => {
                        controller.notice("Não foi possível gravar a exportação.", true)
                    }
                    "follow-started" => {
                        controller.android_set_following(true);
                        bridge.set_following_room(true);
                        bridge.set_room_status("Conectando à sala…".into());
                    }
                    "follow-stopped" => {
                        controller.android_set_following(false);
                        bridge.set_following_room(false);
                        bridge.set_room_status("".into());
                    }
                    "follow-waiting" => {
                        controller.android_wait_for_room();
                        bridge.set_room_status(
                            "Aguardando uma faixa YouTube Music do anfitrião.".into(),
                        );
                    }
                    "follow-failed" => {
                        controller.android_set_following(false);
                        bridge.set_following_room(false);
                        bridge.set_room_status("Não foi possível seguir a sala.".into());
                    }
                    "suggest-received" => {
                        controller.notice("Sugestão recebida pelo anfitrião.", false)
                    }
                    "suggest-unavailable" => {
                        controller.notice("A sala não aceita contribuições à playlist.", true)
                    }
                    "suggest-no-track" => {
                        controller.notice("Toque uma faixa do YouTube Music para enviar.", false)
                    }
                    "lastfm-confirm" => bridge
                        .set_lastfm_status("Autorize no navegador e confirme a conexão.".into()),
                    "lastfm-connected" => {
                        bridge.set_lastfm_connected(true);
                        bridge.set_lastfm_status("Scrobbles ativos".into());
                    }
                    "lastfm-disconnected" => {
                        bridge.set_lastfm_connected(false);
                        bridge.set_lastfm_status("Desconectado".into());
                    }
                    "lastfm-authorize-failed" => {
                        bridge.set_lastfm_status("Não foi possível concluir a autorização.".into())
                    }
                    "lastfm-scrobble-failed" => {
                        controller.notice("O Last.fm não recebeu um scrobble.", true)
                    }
                    "remote-host-failed" => {
                        controller.notice("Rede local indisponível para o controle remoto.", true)
                    }
                    "remote-send-failed" => controller.notice(
                        "Código inválido, expirado ou aparelho fora da rede local.",
                        true,
                    ),
                    "remote-sent" => {
                        controller.notice("Comando recebido pelo outro aparelho.", false)
                    }
                    "share-unavailable" => {
                        controller.notice("Compartilhamento indisponível.", true)
                    }
                    "folder-unavailable" => {
                        controller.notice("Seletor de pastas indisponível.", true)
                    }
                    "folder-permission" => controller
                        .notice("O Android não concedeu acesso persistente à pasta.", true),
                    "folder-import-failed" => {
                        controller.notice("Uma ou mais pastas não puderam ser sincronizadas.", true)
                    }
                    "background-unavailable" => controller
                        .notice("Controles de mídia em segundo plano indisponíveis.", true),
                    "a11y:toggle-play" => controller.android_toggle_play(),
                    "a11y:previous" => controller.previous(),
                    "a11y:next" => controller.next(),
                    "a11y:shuffle" => controller.toggle_shuffle(),
                    "a11y:repeat" => controller.cycle_repeat(),
                    "a11y:lyrics" => controller.android_lyrics(),
                    "a11y:share" => controller.android_share_now(),
                    "a11y:watch" => controller.android_watch_now(),
                    "a11y:detail-play" => controller.android_play_detail(),
                    "a11y:detail-close" => controller.android_close_detail(),
                    _ => {
                        let Some(suffix) = action.strip_prefix("a11y:") else { return };
                        if let Some(tab) = suffix.strip_prefix("navigate:").and_then(|value| value.parse::<i32>().ok()) {
                            controller.android_navigation(tab);
                        } else if let Some(id) = suffix.strip_prefix("play:") {
                            controller.android_play_track(id);
                        } else if let Some(id) = suffix.strip_prefix("like:") {
                            controller.toggle_like(id);
                        } else if let Some(value) = suffix.strip_prefix("seek:").and_then(|value| value.parse::<i64>().ok()) {
                            controller.seek_relative(value);
                        } else if let Some(direction) = suffix.strip_prefix("volume:").and_then(|value| value.parse::<i32>().ok()) {
                            controller.volume_step(direction);
                        } else if let Some(minutes) = suffix.strip_prefix("sleep:").and_then(|value| value.parse::<i32>().ok()) {
                            let minutes = minutes.clamp(0, 120);
                            let duration = (minutes > 0)
                                .then(|| std::time::Duration::from_secs(minutes as u64 * 60));
                            controller.engine().set_sleep_timer(duration);
                            bridge.set_sleep_minutes(minutes);
                        } else if let Some(id) = suffix.strip_prefix("output:") {
                            controller.engine().select_output((!id.is_empty()).then(|| id.to_string()));
                        } else if let Some(query) = suffix.strip_prefix("search:") {
                            controller.search(query);
                        } else if let Some(rest) = suffix.strip_prefix("queue:") {
                            let mut parts = rest.splitn(2, ':');
                            controller.android_queue_action(parts.next().unwrap_or_default(), parts.next().unwrap_or_default());
                        } else if let Some(rest) = suffix.strip_prefix("open:") {
                            let mut parts = rest.splitn(2, ':');
                            controller.android_open_item(parts.next().unwrap_or_default(), parts.next().unwrap_or_default());
                        } else if let Some(rest) = suffix.strip_prefix("setting:") {
                            let mut parts = rest.splitn(2, ':');
                            let value = parts.next().and_then(|value| value.parse::<i32>().ok()).unwrap_or(0);
                            controller.android_setting(parts.next().unwrap_or_default(), value);
                        }
                    }
                }
            });
        });
    }

    pub(crate) fn room_playlist_track(playlist: String, id: String, title: String, artist: String) {
        let window = ACTIVITY_WINDOW
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let Some(window) = window else { return };
        let _ = window.upgrade_in_event_loop(move |_| {
            CONTROLLER.with(|slot| {
                if let Some(controller) = slot.borrow().as_ref() {
                    controller
                        .borrow_mut()
                        .android_room_playlist_track(playlist, id, title, artist);
                }
            });
        });
    }

    pub(crate) fn follow_track(
        id: String,
        title: String,
        artist: String,
        position: i64,
        playing: bool,
    ) {
        let window = ACTIVITY_WINDOW
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let Some(window) = window else { return };
        let _ = window.upgrade_in_event_loop(move |window| {
            CONTROLLER.with(|slot| {
                if let Some(controller) = slot.borrow().as_ref() {
                    let updated = controller
                        .borrow_mut()
                        .android_follow_track(id, title, artist, position, playing);
                    if updated {
                        window
                            .global::<AppBridge>()
                            .set_room_status("Seguindo a reprodução da sala.".into());
                    }
                }
            });
        });
    }

    pub(crate) fn remote_code(code: String) {
        let window = ACTIVITY_WINDOW
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(window) = window {
            let _ = window.upgrade_in_event_loop(move |window| {
                window.global::<AppBridge>().set_lan_code(code.into());
            });
        }
    }

    pub(crate) fn playlist_cover(id: String, bytes: Vec<u8>) {
        let window = ACTIVITY_WINDOW
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let Some(window) = window else { return };
        let _ = window.upgrade_in_event_loop(move |_| {
            CONTROLLER.with(|slot| {
                if let Some(controller) = slot.borrow().as_ref() {
                    controller
                        .borrow_mut()
                        .android_set_playlist_cover(id, bytes);
                }
            });
        });
    }

    pub(crate) fn restore_backup(json: String) {
        let window = ACTIVITY_WINDOW
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let Some(window) = window else { return };
        let _ = window.upgrade_in_event_loop(move |_| {
            CONTROLLER.with(|slot| {
                if let Some(controller) = slot.borrow().as_ref() {
                    controller.borrow_mut().android_restore_backup(json);
                }
            });
        });
    }

    pub(crate) fn sync_peer_playlists(json: String) {
        let window = ACTIVITY_WINDOW
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let Some(window) = window else { return };
        let _ = window.upgrade_in_event_loop(move |_| {
            CONTROLLER.with(|slot| {
                if let Some(controller) = slot.borrow().as_ref() {
                    controller.borrow_mut().android_sync_peer_playlists(json);
                }
            });
        });
    }

    pub(crate) fn import_m3u(title: String, contents: String) {
        let window = ACTIVITY_WINDOW
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let Some(window) = window else { return };
        let _ = window.upgrade_in_event_loop(move |_| {
            CONTROLLER.with(|slot| {
                if let Some(controller) = slot.borrow().as_ref() {
                    controller.borrow_mut().android_import_m3u(title, contents);
                }
            });
        });
    }

    pub(crate) fn set_visible(visible: bool) {
        let window = ACTIVITY_WINDOW
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        if let Some(window) = window.and_then(|window| window.upgrade()) {
            window.global::<AppBridge>().set_app_visible(visible);
        }
    }

    pub(crate) fn flush_config() {
        CONTROLLER.with(|slot| {
            if let Some(controller) = slot.borrow().as_ref() {
                controller.borrow_mut().flush_config();
            }
        });
    }

    pub(crate) fn publish_accessibility() {
        let window = ACTIVITY_WINDOW
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
            .and_then(|window| window.upgrade());
        let Some(window) = window else { return };
        let tree = crate::accessibility::tree(&window.global::<AppBridge>());
        {
            let mut last = LAST_ACCESSIBILITY.lock().unwrap_or_else(|error| error.into_inner());
            if last.as_deref() == Some(tree.as_str()) {
                return;
            }
        }
        if crate::android::accessibility_tree(&tree) {
            *LAST_ACCESSIBILITY.lock().unwrap_or_else(|error| error.into_inner()) = Some(tree);
        }
    }

    pub(crate) fn attach(
        window: &AppWindow,
        paths: AndroidPaths,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let config_path = paths.files.join("config.toml");
        let (config, recovered_config) = match Config::load(&config_path) {
            Ok(config) => (config, false),
            Err(error) => {
                log::warn!("configuração Android ilegível: {error}");
                Config::back_up_corrupt(&config_path)?;
                (Config::default(), true)
            }
        };
        let generation = WINDOW_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;

        // Engine and job callbacks always target the currently attached window.
        let sink: MsgSink = Arc::new(move |message| {
            let window = ACTIVITY_WINDOW
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone();
            let Some(window) = window else { return };
            let _ = window.upgrade_in_event_loop(move |_| {
                CONTROLLER.with(|slot| {
                    let controller = slot.borrow().clone();
                    if let Some(controller) = controller {
                        controller.borrow_mut().handle(message);
                    }
                });
            });
        });

        let (remote, remote_error) = match YtmCatalog::open_with_cache(&paths.files, &paths.cache) {
            Ok(catalog) => (
                Some(RemoteWorker::new(
                    Providers::new().with(Arc::new(catalog)),
                    CoverFetcher::new(Some(paths.cache.join("covers"))),
                )),
                None,
            ),
            Err(error) => (None, Some(error)),
        };
        let local_quality = Arc::new(AtomicI32::new(config.stream_quality.index()));
        let (engine, quality, remote_enabled, fresh_engine) = {
            let mut slot = Self::session_slot()
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if let Some(session) = slot.as_ref() {
                (
                    session.engine.clone(),
                    session.quality.clone(),
                    session.remote,
                    false,
                )
            } else {
                let mut audio = EngineConfig {
                    volume: crate::controller::sane_unit(config.volume, 1.0),
                    ..Default::default()
                };
                audio.stream.cache = Some(CacheConfig {
                    dir: paths.cache.join("streams"),
                    max_bytes: 256 * 1024 * 1024,
                });
                let mut builder = Engine::builder().config(audio);
                let mut engine_remote = false;
                if let Some(catalog) = remote
                    .as_ref()
                    .and_then(|worker| worker.providers().default_remote())
                    .cloned()
                {
                    builder = crate::catalog::install_resolver(builder, catalog, local_quality.clone());
                    engine_remote = true;
                }
                let engine = builder.spawn()?;
                let controller_engine = engine.clone();
                *slot = Some(EngineSession {
                    engine,
                    quality: local_quality.clone(),
                    remote: engine_remote,
                });
                (controller_engine, local_quality, engine_remote, true)
            }
        };
        engine.set_crossfade(std::time::Duration::from_secs(
            config.mobile_appearance.crossfade_seconds as u64,
        ));
        if fresh_engine {
            spawn_engine_bridge(&engine, sink.clone())?;
            spawn_stream_bridge(&engine, sink.clone())?;
        }
        if quality.load(Ordering::SeqCst) != config.stream_quality.index() {
            quality.store(config.stream_quality.index(), Ordering::SeqCst);
        }

        let emit = sink.clone();
        let worker = ThreadedRunner::spawn_with_cache(
            paths.files.clone(),
            paths.cache.clone(),
            Arc::new(move |result| emit(Msg::Job(result))),
            remote,
        )?;
        let mut controller = Controller::new(
            AndroidView::new(window),
            engine,
            Box::new(worker),
            sink,
            config,
            Some(config_path),
        );
        controller.set_data_dir(paths.files);
        controller.set_cache_dir(paths.cache);
        if remote_enabled {
            controller.enable_engine_remote(quality);
        }
        let controller = Rc::new(RefCell::new(controller));
        let bridge = window.global::<AppBridge>();
        bridge.set_version(env!("CARGO_PKG_VERSION").into());

        macro_rules! bind {
            ($callback:ident, $method:ident $(, $arg:ident $(=> $value:expr)?)*) => {{
                let weak = Rc::downgrade(&controller);
                bridge.$callback(move |$($arg),*| {
                    if let Some(controller) = weak.upgrade() {
                        controller.borrow_mut().$method($($value),*);
                    }
                });
            }};
        }

        bind!(on_play_track, android_play_track, id => id.as_str());
        bind!(on_toggle_play, android_toggle_play);
        bind!(on_next, next);
        bind!(on_previous, previous);
        bind!(on_seek, seek, value => value);
        bind!(on_set_volume, set_volume, value => value);
        bind!(on_toggle_shuffle, toggle_shuffle);
        bind!(on_cycle_repeat, cycle_repeat);
        bind!(on_toggle_like, toggle_like, id => id.as_str());
        bind!(on_search, search, query => query.as_str());
        bind!(on_clear_search, clear_recent_searches);
        bind!(on_queue_action, android_queue_action, action => action.as_str(), id => id.as_str());
        bind!(on_search_source, search_source_changed, source => source);
        bind!(on_create_playlist, android_create_playlist, name => name.as_str());
        bind!(on_rename_playlist, android_rename_playlist, name => name.as_str());
        bind!(on_delete_playlist, android_delete_playlist);
        bind!(on_add_now_to_playlist, android_add_now_to_playlist, id => id.as_str());
        bind!(on_edit_playlist_row, android_edit_playlist_row, action => action.as_str(), index => index, id => id.as_str());
        bind!(on_share_now, android_share_now);
        bind!(on_lyrics_request, android_lyrics);
        bind!(on_notice_pressed, toast_action);
        bind!(on_watch_now, android_watch_now);
        bridge.on_lastfm_authorize(|key, secret| {
            crate::android::last_fm_authorize(key.as_str(), secret.as_str())
        });
        bridge.on_lastfm_finish(crate::android::last_fm_finish);
        bind!(on_host_remote, android_host_room, enabled => enabled, share => share, playlist => playlist.as_str());
        bridge.on_suggest_room_track(|code| crate::android::suggest_current_track(code.as_str()));
        bridge.on_copy_peer_backup(|code| crate::android::copy_backup_from_peer(code.as_str()));
        bridge.on_follow_room(|code| crate::android::follow_room(code.as_str()));
        bridge.on_send_remote(|code, command| crate::android::send_remote(code.as_str(), command));
        bind!(on_edit_tags, android_edit_tags, id => id.as_str(), title => title.as_str(), artist => artist.as_str(), album => album.as_str(), genre => genre.as_str());
        bind!(on_listening_stats, android_stats, period => period, export => export);
        bind!(on_import_account, import_account, provider => provider.to_string(), token => token.to_string());
        bind!(on_track_queue, android_queue_track, id => id.as_str(), next => next);
        bind!(on_home_feed, android_home_feed, feed => feed);
        bind!(on_open_item, android_open_item, kind => kind.as_str(), id => id.as_str());
        bind!(on_close_detail, android_close_detail);
        bind!(on_remove_cached, android_remove_cached, id => id.as_str());
        bind!(on_play_detail, android_play_detail);
        bind!(on_navigation, android_navigation, tab => tab);

        let weak = Rc::downgrade(&controller);
        let window_for_outputs = window.as_weak();
        bridge.on_list_outputs(move || {
            let window = window_for_outputs.clone();
            if weak.upgrade().is_none() {
                return;
            }
            let _ = std::thread::Builder::new()
                .name("kazzu-outputs".into())
                .spawn(move || {
                    let outputs = kazzu_audio::CpalSink::devices();
                    let _ = window.upgrade_in_event_loop(move |window| {
                        let rows = std::iter::once(crate::ui::Tile {
                            title: "Padrão do sistema".into(),
                            ..Default::default()
                        })
                        .chain(outputs.into_iter().map(|device| crate::ui::Tile {
                            id: device.id.into(),
                            title: device.name.into(),
                            ..Default::default()
                        }))
                        .collect::<Vec<_>>();
                        window
                            .global::<AppBridge>()
                            .set_outputs(slint::ModelRc::new(slint::VecModel::from(rows)));
                    });
                });
        });
        let weak = Rc::downgrade(&controller);
        bridge.on_select_output(move |id| {
            if let Some(controller) = weak.upgrade() {
                controller
                    .borrow()
                    .engine()
                    .select_output((!id.is_empty()).then(|| id.to_string()));
            }
        });
        let weak = Rc::downgrade(&controller);
        let timer_window = window.as_weak();
        bridge.on_sleep_timer(move |minutes| {
            let (Some(controller), Some(window)) = (weak.upgrade(), timer_window.upgrade()) else {
                return;
            };
            let minutes = minutes.clamp(0, 120);
            let duration =
                (minutes > 0).then(|| std::time::Duration::from_secs(minutes as u64 * 60));
            controller.borrow().engine().set_sleep_timer(duration);
            window.global::<AppBridge>().set_sleep_minutes(minutes);
        });
        bind!(on_glass_change, android_glass, index => index, value => value);
        bind!(on_setting, android_setting, name => name.as_str(), value => value);

        *ACTIVITY_WINDOW.lock().unwrap_or_else(|e| e.into_inner()) = Some(window.as_weak());
        CONTROLLER.with(|slot| *slot.borrow_mut() = Some(controller.clone()));
        controller
            .borrow_mut()
            .init(crate::platform::system_prefs());
        if let Some(state) = controller.borrow().engine().state() {
            controller.borrow_mut().android_restore_state(state);
        }
        controller.borrow_mut().android_register_imports();
        crate::android::last_fm_status();
        if recovered_config {
            controller.borrow_mut().notice(
                "Configuração recuperada. A cópia anterior foi preservada.",
                true,
            );
        }
        if let Some(error) = remote_error {
            log::warn!("YouTube Music indisponível: {error}");
            controller
                .borrow_mut()
                .notice("YouTube Music indisponível.", true);
        }
        *LAST_ACCESSIBILITY.lock().unwrap_or_else(|e| e.into_inner()) = None;
        Self::publish_accessibility();
        Ok(Self {
            generation,
        })
    }
}

impl Drop for AndroidBackend {
    fn drop(&mut self) {
        if WINDOW_GENERATION.load(Ordering::SeqCst) != self.generation {
            return;
        }
        ACTIVITY_WINDOW
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(controller) = CONTROLLER.with(|slot| slot.borrow_mut().take()) {
            controller.borrow_mut().flush_config();
        }
    }
}
