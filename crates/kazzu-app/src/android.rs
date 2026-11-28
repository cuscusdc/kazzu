//! Ponto de entrada Android (NativeActivity via `android-activity` + backend Slint).
//! O script empacota a cdylib e a ponte Java do sistema no APK.
//! A interface de produção é a v6 mobile (`ui-mobile-v6`).

use slint::ComponentHandle;
use slint::android::AndroidApp;

use crate::android_app::AndroidBackend;
use crate::android_paths::AndroidPaths;

thread_local! {
    static ACTIVITY: std::cell::RefCell<Option<AndroidApp>> = const { std::cell::RefCell::new(None) };
}

fn with_activity<T>(
    action: impl FnOnce(&mut jni::JNIEnv<'_>, jni::objects::JObject<'_>) -> jni::errors::Result<T>,
) -> Option<T> {
    ACTIVITY.with(|slot| {
        let slot = slot.borrow();
        let app = slot.as_ref()?;
        // AndroidApp owns the global references until this scope returns.
        let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) }.ok()?;
        let mut env = vm.attach_current_thread().ok()?;
        let result = env.with_local_frame(12, |env| {
            let activity = unsafe { jni::objects::JObject::from_raw(app.activity_as_ptr().cast()) };
            action(env, activity)
        });
        if matches!(&result, Err(jni::errors::Error::JavaException)) {
            let _ = env.exception_clear();
        }
        if let Err(ref error) = result {
            log::warn!("ponte Android: {error}");
        }
        result.ok()
    })
}

pub(crate) fn open_web_url(url: &str) -> bool {
    with_activity(|env, activity| {
        use jni::objects::JValue;
        let url = env.new_string(url)?;
        let uri = env.call_static_method(
            "android/net/Uri", "parse", "(Ljava/lang/String;)Landroid/net/Uri;",
            &[JValue::Object(&url)],
        )?.l()?;
        let action = env.new_string("android.intent.action.VIEW")?;
        let intent = env.new_object(
            "android/content/Intent", "(Ljava/lang/String;Landroid/net/Uri;)V",
            &[JValue::Object(&action), JValue::Object(&uri)],
        )?;
        env.call_method(activity, "startActivity", "(Landroid/content/Intent;)V", &[JValue::Object(&intent)])?;
        Ok(())
    }).is_some()
}

pub(crate) fn listening_period_start(period: i32) -> Option<i64> {
    with_activity(|env, activity| {
        Ok(env.call_method(activity, "listeningPeriodStart", "(I)J", &[jni::objects::JValue::Int(period)])?.j()?)
    })
}

pub(crate) fn export_listening_stats(json: &str) -> bool {
    with_activity(|env, activity| {
        let json = env.new_string(json)?;
        env.call_method(activity, "exportListeningStats", "(Ljava/lang/String;)V", &[jni::objects::JValue::Object(&json)])?;
        Ok(())
    }).is_some()
}

pub(crate) fn open_video(id: &str) -> bool {
    with_activity(|env, activity| {
        let id = env.new_string(id)?;
        env.call_method(activity, "openVideo", "(Ljava/lang/String;)V", &[jni::objects::JValue::Object(&id)])?;
        Ok(())
    }).is_some()
}

pub(crate) fn share_text(text: &str) -> bool {
    with_activity(|env, activity| {
        use jni::objects::JValue;
        let text = env.new_string(text)?;
        env.call_method(activity, "shareTrack", "(Ljava/lang/String;)V", &[JValue::Object(&text)])?;
        Ok(())
    }).is_some()
}

pub(crate) fn select_music_folder() -> bool {
    with_activity(|env, activity| {
        env.call_method(activity, "selectMusicFolder", "()V", &[])?;
        Ok(())
    }).is_some()
}

pub(crate) fn select_playlist_cover(id: &str) -> bool {
    with_activity(|env, activity| {
        let id = env.new_string(id)?;
        env.call_method(activity, "selectPlaylistCover", "(Ljava/lang/String;)V", &[jni::objects::JValue::Object(&id)])?;
        Ok(())
    }).is_some()
}

#[unsafe(no_mangle)]
extern "system" fn Java_app_kazzu_player_KazzuActivity_nativePlaylistCover(
    mut env: jni::JNIEnv<'_>,
    _class: jni::objects::JClass<'_>,
    id: jni::objects::JString<'_>,
    bytes: jni::objects::JByteArray<'_>,
) {
    let Ok(id) = env.get_string(&id).map(String::from) else { return };
    let Ok(bytes) = env.convert_byte_array(bytes) else { return };
    AndroidBackend::playlist_cover(id, bytes);
}

pub(crate) fn host_backup(json: &str) -> bool {
    with_activity(|env, activity| {
        let json = env.new_string(json)?;
        env.call_method(activity, "hostBackup", "(Ljava/lang/String;)V", &[jni::objects::JValue::Object(&json)])?;
        Ok(())
    }).is_some()
}

pub(crate) fn copy_backup_from_peer(code: &str) {
    with_activity(|env, activity| {
        let code = env.new_string(code)?;
        env.call_method(activity, "copyBackupFromPeer", "(Ljava/lang/String;)V", &[jni::objects::JValue::Object(&code)])?;
        Ok(())
    });
}

pub(crate) fn export_backup(json: &str) -> bool {
    with_activity(|env, activity| {
        let json = env.new_string(json)?;
        env.call_method(activity, "exportBackup", "(Ljava/lang/String;)V", &[jni::objects::JValue::Object(&json)])?;
        Ok(())
    }).is_some()
}

pub(crate) fn publish_room_backup(json: &str) -> bool {
    with_activity(|env, activity| {
        let json = env.new_string(json)?;
        env.call_method(activity, "receiveRoomBackup", "(Ljava/lang/String;)V", &[jni::objects::JValue::Object(&json)])?;
        Ok(())
    }).is_some()
}

pub(crate) fn select_backup() -> bool {
    with_activity(|env, activity| {
        env.call_method(activity, "selectBackup", "()V", &[])?;
        Ok(())
    }).is_some()
}

#[unsafe(no_mangle)]
extern "system" fn Java_app_kazzu_player_KazzuActivity_nativeRestoreBackup(
    mut env: jni::JNIEnv<'_>,
    _class: jni::objects::JClass<'_>,
    json: jni::objects::JString<'_>,
) {
    if let Ok(json) = env.get_string(&json) {
        AndroidBackend::restore_backup(String::from(json));
    }
}

pub(crate) fn select_m3u() -> bool {
    with_activity(|env, activity| {
        env.call_method(activity, "selectM3u", "()V", &[])?;
        Ok(())
    }).is_some()
}

#[unsafe(no_mangle)]
extern "system" fn Java_app_kazzu_player_KazzuActivity_nativeSyncPeerPlaylists(
    mut env: jni::JNIEnv<'_>,
    _class: jni::objects::JClass<'_>,
    json: jni::objects::JString<'_>,
) {
    if let Ok(json) = env.get_string(&json) {
        AndroidBackend::sync_peer_playlists(String::from(json));
    }
}

#[unsafe(no_mangle)]
extern "system" fn Java_app_kazzu_player_KazzuActivity_nativeImportM3u(
    mut env: jni::JNIEnv<'_>,
    _class: jni::objects::JClass<'_>,
    title: jni::objects::JString<'_>,
    contents: jni::objects::JString<'_>,
) {
    let Ok(title) = env.get_string(&title).map(String::from) else { return };
    let Ok(contents) = env.get_string(&contents).map(String::from) else { return };
    AndroidBackend::import_m3u(title, contents);
}

pub(crate) fn last_fm_status() {
    with_activity(|env, activity| {
        env.call_method(activity, "lastFmStatus", "()V", &[])?;
        Ok(())
    });
}

pub(crate) fn last_fm_authorize(api_key: &str, secret: &str) {
    use jni::objects::JValue;
    let api_key = api_key.trim();
    let secret = secret.trim();
    if [api_key, secret].into_iter().any(|value| value.len() != 32 || !value.bytes().all(|byte| byte.is_ascii_hexdigit())) {
        AndroidBackend::media_action("lastfm-authorize-failed".into(), 0);
        return;
    }
    with_activity(|env, activity| {
        let api_key = env.new_string(api_key)?;
        let secret = env.new_string(secret)?;
        env.call_method(activity, "lastFmAuthorize", "(Ljava/lang/String;Ljava/lang/String;)V", &[
            JValue::Object(&api_key), JValue::Object(&secret),
        ])?;
        Ok(())
    });
}

pub(crate) fn last_fm_finish(connect: bool) {
    with_activity(|env, activity| {
        env.call_method(activity, "lastFmFinish", "(Z)V", &[jni::objects::JValue::Bool(u8::from(connect))])?;
        Ok(())
    });
}

pub(crate) fn last_fm_scrobble(title: &str, artist: &str, album: &str, started: i64, duration: i64) {
    use jni::objects::JValue;
    with_activity(|env, activity| {
        let title = env.new_string(title)?;
        let artist = env.new_string(artist)?;
        let album = env.new_string(album)?;
        env.call_method(activity, "lastFmScrobble", "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;JJ)V",
            &[JValue::Object(&title), JValue::Object(&artist), JValue::Object(&album), JValue::Long(started), JValue::Long(duration)])?;
        Ok(())
    });
}

pub(crate) fn host_remote(enabled: bool, share: bool, playlist: &str) {
    use jni::objects::JValue;
    with_activity(|env, activity| {
        let playlist = env.new_string(playlist)?;
        env.call_method(activity, "hostRemote", "(ZZLjava/lang/String;)V", &[
            JValue::Bool(u8::from(enabled)), JValue::Bool(u8::from(share)), JValue::Object(&playlist),
        ])?;
        Ok(())
    });
}

pub(crate) fn suggest_current_track(code: &str) {
    with_activity(|env, activity| {
        let code = env.new_string(code)?;
        env.call_method(activity, "suggestCurrentTrack", "(Ljava/lang/String;)V", &[jni::objects::JValue::Object(&code)])?;
        Ok(())
    });
}

#[unsafe(no_mangle)]
extern "system" fn Java_app_kazzu_player_KazzuActivity_nativeRoomPlaylistTrack(
    mut env: jni::JNIEnv<'_>,
    _class: jni::objects::JClass<'_>,
    playlist: jni::objects::JString<'_>,
    id: jni::objects::JString<'_>,
    title: jni::objects::JString<'_>,
    artist: jni::objects::JString<'_>,
) {
    let Ok(playlist) = env.get_string(&playlist).map(String::from) else { return };
    let Ok(id) = env.get_string(&id).map(String::from) else { return };
    let Ok(title) = env.get_string(&title).map(String::from) else { return };
    let Ok(artist) = env.get_string(&artist).map(String::from) else { return };
    AndroidBackend::room_playlist_track(playlist, id, title, artist);
}

pub(crate) fn follow_room(code: &str) {
    with_activity(|env, activity| {
        let code = env.new_string(code)?;
        env.call_method(activity, "followRoom", "(Ljava/lang/String;)V", &[jni::objects::JValue::Object(&code)])?;
        Ok(())
    });
}

#[unsafe(no_mangle)]
extern "system" fn Java_app_kazzu_player_KazzuActivity_nativeFollowTrack(
    mut env: jni::JNIEnv<'_>,
    _class: jni::objects::JClass<'_>,
    id: jni::objects::JString<'_>,
    title: jni::objects::JString<'_>,
    artist: jni::objects::JString<'_>,
    position: jni::sys::jlong,
    playing: jni::sys::jboolean,
) {
    let Ok(id) = env.get_string(&id).map(String::from) else { return };
    let Ok(title) = env.get_string(&title).map(String::from) else { return };
    let Ok(artist) = env.get_string(&artist).map(String::from) else { return };
    AndroidBackend::follow_track(id, title, artist, position, playing != 0);
}

pub(crate) fn send_remote(code: &str, command: i32) {
    use jni::objects::JValue;
    with_activity(|env, activity| {
        let code = env.new_string(code)?;
        env.call_method(activity, "sendRemote", "(Ljava/lang/String;I)V", &[JValue::Object(&code), JValue::Int(command)])?;
        Ok(())
    });
}

#[unsafe(no_mangle)]
extern "system" fn Java_app_kazzu_player_KazzuActivity_nativeRemoteCode(
    mut env: jni::JNIEnv<'_>,
    _class: jni::objects::JClass<'_>,
    code: jni::objects::JString<'_>,
) {
    if let Ok(code) = env.get_string(&code) {
        AndroidBackend::remote_code(String::from(code));
    }
}

pub(crate) fn find_duplicates() {
    with_activity(|env, activity| {
        env.call_method(activity, "findDuplicates", "()V", &[])?;
        Ok(())
    });
}

pub(crate) fn refresh_music_folders() {
    with_activity(|env, activity| {
        env.call_method(activity, "refreshMusicFolders", "()V", &[])?;
        Ok(())
    });
}

pub(crate) fn prefers_reduced_motion() -> Option<bool> {
    with_activity(|env, activity| env.call_method(activity, "prefersReducedMotion", "()Z", &[])?.z())
}

pub(crate) fn request_playback_focus() -> bool {
    with_activity(|env, activity| env.call_method(activity, "requestPlaybackFocus", "()Z", &[])?.z())
        .unwrap_or(false)
}

pub(crate) fn publish_playback(id: &str, title: &str, artist: &str, duration: i64, position: i64, playing: bool, buffering: bool) {
    use jni::objects::JValue;
    with_activity(|env, activity| {
        let id = env.new_string(id)?;
        let title = env.new_string(title)?;
        let artist = env.new_string(artist)?;
        env.call_method(
            activity, "publishPlayback", "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;JJZZ)V",
            &[
                JValue::Object(&id), JValue::Object(&title), JValue::Object(&artist), JValue::Long(duration),
                JValue::Long(position), JValue::Bool(u8::from(playing)), JValue::Bool(u8::from(buffering)),
            ],
        )?;
        Ok(())
    });
}

pub(crate) fn accessibility_tree(json: &str) -> bool {
    with_activity(|env, activity| {
        let json = env.new_string(json)?;
        env.call_method(activity, "updateAccessibilityTree", "(Ljava/lang/String;)V", &[
            jni::objects::JValue::Object(&json),
        ])?;
        Ok(())
    }).is_some()
}

#[unsafe(no_mangle)]
extern "system" fn Java_app_kazzu_player_KazzuActivity_nativeMediaAction(
    mut env: jni::JNIEnv<'_>,
    _class: jni::objects::JClass<'_>,
    action: jni::objects::JString<'_>,
    position_ms: jni::sys::jlong,
) {
    if let Ok(action) = env.get_string(&action) {
        AndroidBackend::media_action(String::from(action), position_ms);
    }
}

#[unsafe(no_mangle)]
extern "system" fn Java_app_kazzu_player_KazzuActivity_nativeShutdown(
    _env: jni::JNIEnv<'_>,
    _class: jni::objects::JClass<'_>,
) {
    AndroidBackend::shutdown_engine();
}

#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("kazzu"),
    );

    use slint::android::android_activity::{MainEvent, PollEvent};
    if let Err(e) = slint::android::init_with_event_listener(app.clone(), |event| {
        match event {
            PollEvent::Main(MainEvent::Pause) => {
                AndroidBackend::set_visible(false);
                AndroidBackend::flush_config();
            }
            PollEvent::Main(MainEvent::Resume { .. }) => AndroidBackend::set_visible(true),
            _ => {}
        }
    }) {
        log::error!("não foi possível iniciar o backend Android do Slint: {e}");
        return;
    }

    let window = match crate::ui::AppWindow::new() {
        Ok(window) => window,
        Err(e) => {
            log::error!("não foi possível criar a janela Kazzu: {e}");
            return;
        }
    };

    let rect = app.content_rect();
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    if width > 0 && height > 0 {
        window.window().set_size(slint::PhysicalSize::new(width as u32, height as u32));
    }

    ACTIVITY.with(|slot| *slot.borrow_mut() = Some(app.clone()));
    let backend = match AndroidPaths::from_app(&app)
        .map_err(|error| -> Box<dyn std::error::Error> { Box::new(error) })
        .and_then(|paths| AndroidBackend::attach(&window, paths))
    {
        Ok(backend) => Some(backend),
        Err(error) => {
            log::error!("backend Android indisponível: {error}");
            window.global::<crate::ui::AppBridge>().set_startup_error("Não foi possível iniciar o player.".into());
            None
        }
    };

    if let Err(e) = window.run() {
        log::error!("kazzu: {e}");
    }
    drop(backend);
    ACTIVITY.with(|slot| slot.borrow_mut().take());
}
