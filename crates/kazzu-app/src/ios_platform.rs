//! iOS equivalents of the small platform surface used by the shared backend.
//! The platform does not expose Android intents or a system media session;
//! these functions keep the shared controller portable and fail safely.

#[allow(dead_code)]
pub(crate) fn open_web_url(_url: &str) -> bool {
    false
}

#[allow(dead_code)]
pub(crate) fn listening_period_start(_period: i32) -> Option<i64> {
    None
}

#[allow(dead_code)]
pub(crate) fn export_listening_stats(_json: &str) -> bool {
    false
}

#[allow(dead_code)]
pub(crate) fn open_video(_id: &str) -> bool {
    false
}

#[allow(dead_code)]
pub(crate) fn share_text(_text: &str) -> bool {
    false
}

#[allow(dead_code)]
pub(crate) fn select_music_folder() -> bool {
    false
}

#[allow(dead_code)]
pub(crate) fn select_playlist_cover(_id: &str) -> bool {
    false
}

#[allow(dead_code)]
pub(crate) fn host_backup(_json: &str) -> bool {
    false
}

#[allow(dead_code)]
pub(crate) fn copy_backup_from_peer(_code: &str) {}

#[allow(dead_code)]
pub(crate) fn export_backup(_json: &str) -> bool {
    false
}

#[allow(dead_code)]
pub(crate) fn publish_room_backup(_json: &str) -> bool {
    false
}

#[allow(dead_code)]
pub(crate) fn select_backup() -> bool {
    false
}

#[allow(dead_code)]
pub(crate) fn select_m3u() -> bool {
    false
}

#[allow(dead_code)]
pub(crate) fn last_fm_status() {}

#[allow(dead_code)]
pub(crate) fn last_fm_authorize(_api_key: &str, _secret: &str) {}

#[allow(dead_code)]
pub(crate) fn last_fm_finish(_connect: bool) {}

#[allow(dead_code)]
pub(crate) fn last_fm_scrobble(
    _title: &str,
    _artist: &str,
    _album: &str,
    _started: i64,
    _duration: i64,
) {
}

#[allow(dead_code)]
pub(crate) fn host_remote(_enabled: bool, _share: bool, _playlist: &str) {}

#[allow(dead_code)]
pub(crate) fn suggest_current_track(_code: &str) {}

#[allow(dead_code)]
pub(crate) fn follow_room(_code: &str) {}

#[allow(dead_code)]
pub(crate) fn send_remote(_code: &str, _command: i32) {}

#[allow(dead_code)]
pub(crate) fn find_duplicates() {}

#[allow(dead_code)]
pub(crate) fn refresh_music_folders() {}

#[allow(dead_code)]
pub(crate) fn prefers_reduced_motion() -> Option<bool> {
    None
}

#[allow(dead_code)]
pub(crate) fn request_playback_focus() -> bool {
    true
}

#[allow(dead_code)]
pub(crate) fn publish_playback(
    _id: &str,
    _title: &str,
    _artist: &str,
    _duration: i64,
    _position: i64,
    _playing: bool,
    _buffering: bool,
) {
}

#[allow(dead_code)]
pub(crate) fn accessibility_tree(_json: &str) -> bool {
    false
}
