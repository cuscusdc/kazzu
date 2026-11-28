//! Projects shared backend view models onto the mobile v6 contract.

use slint::{ComponentHandle, Image, ModelRc, VecModel};

use crate::accent::Accent;
use crate::art::Pixels;
use crate::config::ThemePref;
use crate::ui::{AppBridge, AppWindow, Tile, Track};
use crate::view::View;
use crate::vm::*;

pub(crate) struct AndroidView {
    window: slint::Weak<AppWindow>,
    last_position: std::cell::Cell<u64>,
}

impl AndroidView {
    pub(crate) fn new(window: &AppWindow) -> Self {
        Self {
            window: window.as_weak(),
            last_position: std::cell::Cell::new(0),
        }
    }

    fn update(&self, update: impl FnOnce(AppBridge<'_>)) {
        if let Some(window) = self.window.upgrade() {
            update(window.global::<AppBridge>());
            drop(window);
            crate::android_app::AndroidBackend::publish_accessibility();
        }
    }

    fn publish_media(&self, bridge: &AppBridge<'_>) {
        let now = bridge.get_now();
        crate::android::publish_playback(
            now.id.as_str(),
            now.title.as_str(),
            now.artist.as_str(),
            i64::from(now.duration) * 1000,
            i64::from(bridge.get_position_ms()),
            bridge.get_playing(),
            bridge.get_buffering(),
        );
    }
}

fn image(pixels: Option<&Pixels>) -> Image {
    pixels.cloned().map(Image::from_rgba8).unwrap_or_default()
}

fn track(t: &TrackVm) -> Track {
    Track {
        id: t.id.as_str().into(),
        title: t.title.as_str().into(),
        artist: t.artist.as_str().into(),
        album: t.album.as_str().into(),
        duration_label: t.duration_label.as_str().into(),
        local: t.source.is_empty(),
        cover: image(t.cover.as_ref()),
        ..Default::default()
    }
}

fn tracks(values: &[TrackVm]) -> ModelRc<Track> {
    ModelRc::new(VecModel::from(values.iter().map(track).collect::<Vec<_>>()))
}

fn color([r, g, b]: [u8; 3]) -> slint::Color {
    slint::Color::from_rgb_u8(r, g, b)
}

fn tiles(values: impl IntoIterator<Item = Tile>) -> ModelRc<Tile> {
    ModelRc::new(VecModel::from(values.into_iter().collect::<Vec<_>>()))
}

fn highlight(item: &HighlightVm) -> Tile {
    Tile {
        id: item.id.as_str().into(),
        kind: item.kind.as_str().into(),
        title: item.title.as_str().into(),
        sub: item.subtitle.as_str().into(),
        cover: image(item.cover.as_ref()),
        tone: item.tint.map(color).unwrap_or_else(|| color([54, 49, 45])),
        glyph: "♪".into(),
    }
}

impl View for AndroidView {
    fn set_playback(&self, playback: &PlaybackVm) {
        self.update(|bridge| {
            let previous = bridge.get_now();
            let mut now = playback.now.as_ref().map(track).unwrap_or_default();
            if previous.id == now.id {
                now.glow = previous.glow;
                now.cover = previous.cover;
                now.tint = previous.tint;
                now.art = previous.art;
                now.duration = previous.duration;
                now.lyrics = previous.lyrics;
                now.lyric_times = previous.lyric_times;
            } else {
                bridge.set_lyrics_text("".into());
                bridge.set_lyrics_attribution("".into());
                bridge.set_lyrics_loading(false);
            }
            bridge.set_now(now);
            bridge.set_has_now(playback.has_now);
            bridge.set_playing(playback.playing);
            bridge.set_buffering(playback.loading);
            bridge.set_liked(playback.now.as_ref().is_some_and(|t| t.liked));
            bridge.set_shuffle(playback.shuffle);
            bridge.set_repeat(playback.repeat != RepeatVm::Off);
            bridge.set_volume(playback.volume);
            bridge.set_output_name(playback.output_name.as_str().into());
            bridge.set_bit_perfect(playback.has_now && playback.bit_perfect);
            self.publish_media(&bridge);
        });
    }

    fn set_seek_available(&self, available: bool) {
        self.update(|bridge| bridge.set_seekable(available));
    }

    fn set_position_millis(&self, position: u64, duration: Option<u64>) {
        self.update(|bridge| {
            bridge.set_position_ms(position.min(i32::MAX as u64) as i32);
            let mut now = bridge.get_now();
            let duration = (duration.unwrap_or_default() / 1000).min(i32::MAX as u64) as i32;
            if now.duration != duration {
                now.duration = duration;
                bridge.set_now(now);
            }
            if self.last_position.replace(position / 1000) != position / 1000 {
                self.publish_media(&bridge);
            }
        });
    }

    fn set_progress(&self, progress: f32, position: &str, remaining: &str) {
        self.update(|bridge| {
            bridge.set_progress(progress);
            bridge.set_position(position.into());
            bridge.set_remaining(remaining.into());
        });
    }

    fn set_lyrics(&self, lines: &[(i32, String)]) {
        self.update(|bridge| {
            let mut now = bridge.get_now();
            now.lyric_times = ModelRc::new(VecModel::from(
                lines.iter().map(|(ms, _)| *ms).collect::<Vec<_>>(),
            ));
            now.lyrics = ModelRc::new(VecModel::from(
                lines
                    .iter()
                    .map(|(_, text)| slint::SharedString::from(text.as_str()))
                    .collect::<Vec<_>>(),
            ));
            bridge.set_now(now);
        });
    }

    fn set_remote_lyrics(&self, text: &str, attribution: &str, loading: bool) {
        self.update(|bridge| {
            bridge.set_lyrics_text(text.into());
            bridge.set_lyrics_attribution(attribution.into());
            bridge.set_lyrics_loading(loading);
        });
    }

    fn set_now_art(&self, art: &NowArtVm) {
        self.update(|bridge| {
            let mut now = bridge.get_now();
            now.cover = image(art.cover.as_ref().or(art.thumb.as_ref()));
            now.glow = image(art.backdrop_wide.as_ref());
            bridge.set_now(now);
        });
    }

    fn set_ambient(&self, _backdrop: Option<&Pixels>) {}
    fn set_accent(&self, accent: &Accent, dynamic: bool) {
        let accent = if dynamic { *accent } else { Accent::BRAND };
        self.update(|bridge| {
            bridge.set_dark_accent(color(accent.dark.accent));
            bridge.set_dark_ink(color(accent.dark.ink));
            bridge.set_light_accent(color(accent.light.accent));
            bridge.set_light_ink(color(accent.light.ink));
            let mut now = bridge.get_now();
            now.tint = color(accent.dark.accent).darker(0.8);
            now.art = color(accent.dark.accent);
            bridge.set_now(now);
        });
    }

    fn set_home(&self, home: &HomeVm) {
        self.update(|bridge| {
            bridge.set_quick_picks(tracks(&home.quick_picks));
            bridge.set_recents(tracks(&home.recents));
            bridge.set_suggestions(tiles(home.mixes.iter().map(highlight)));
            bridge.set_home_loading(home.state == LoadStateVm::Loading);
        });
    }

    fn set_explore(&self, explore: &ExploreVm) {
        self.update(|bridge| {
            bridge.set_trending(tracks(&explore.trending));
            bridge.set_new_releases(tiles(explore.new_releases.iter().map(|album| Tile {
                id: album.id.as_str().into(),
                kind: "album".into(),
                title: album.title.as_str().into(),
                sub: album.artist.as_str().into(),
                cover: image(album.cover.as_ref()),
                tone: color([54, 49, 45]),
                glyph: "♪".into(),
            })));
            bridge.set_categories(tiles(explore.moods.iter().map(highlight)));
            bridge.set_explore_loading(explore.state == LoadStateVm::Loading);
        });
    }

    fn set_library(&self, library: &LibraryVm) {
        self.update(|bridge| {
            bridge.set_playlists(tiles(
                library
                    .collections
                    .iter()
                    .filter(|item| item.id == "liked")
                    .map(|item| Tile {
                        id: item.id.as_str().into(),
                        kind: "liked".into(),
                        title: item.title.as_str().into(),
                        sub: item.count_label.as_str().into(),
                        cover: image(item.covers.first()),
                        glyph: "♥".into(),
                        tone: color([80, 47, 97]),
                    })
                    .chain(library.playlists.iter().map(|item| Tile {
                        id: item.id.as_str().into(),
                        kind: "playlist".into(),
                        title: item.name.as_str().into(),
                        sub: item.count_label.as_str().into(),
                        cover: image(item.covers.first()),
                        tone: color([54, 49, 45]),
                        glyph: "♪".into(),
                    })),
            ));
            bridge.set_library_albums(tiles(library.albums.iter().map(|item| Tile {
                id: item.id.as_str().into(),
                kind: "album".into(),
                title: item.title.as_str().into(),
                sub: item.artist.as_str().into(),
                cover: image(item.cover.as_ref()),
                tone: color([54, 49, 45]),
                glyph: "♪".into(),
            })));
            bridge.set_library_genres(ModelRc::new(VecModel::from(
                library
                    .genres
                    .iter()
                    .map(|name| slint::SharedString::from(name.as_str()))
                    .collect::<Vec<_>>(),
            )));
            bridge.set_artists(tiles(library.artists.iter().map(|item| {
                Tile {
                    id: item.id.as_str().into(),
                    kind: "artist".into(),
                    title: item.name.as_str().into(),
                    sub: item.subtitle.as_str().into(),
                    cover: image(item.cover.as_ref()),
                    tone: color([54, 49, 45]),
                    glyph: item
                        .name
                        .chars()
                        .next()
                        .map(|c| c.to_uppercase().to_string())
                        .unwrap_or_default()
                        .into(),
                }
            })));
        });
    }
    fn set_listening_stats(&self, stats: &kazzu_library::ListeningStats) {
        self.update(|bridge| {
            let ranks = |values: &[kazzu_library::ListeningRank]| {
                tiles(values.iter().map(|rank| Tile {
                    title: rank.title.as_str().into(),
                    sub: if rank.subtitle.is_empty() {
                        format!("{} reproduções", rank.plays).into()
                    } else {
                        format!("{} · {} reproduções", rank.subtitle, rank.plays).into()
                    },
                    ..Default::default()
                }))
            };
            bridge
                .set_listening_minutes(crate::format::thousands(stats.listened_ms / 60_000).into());
            bridge.set_listening_plays(
                format!("{} reproduções", crate::format::thousands(stats.plays)).into(),
            );
            bridge.set_top_songs(ranks(&stats.songs));
            bridge.set_top_artists(ranks(&stats.artists));
            bridge.set_top_albums(ranks(&stats.albums));
            bridge.set_top_genres(ranks(&stats.genres));
        });
    }

    fn set_settings(&self, _settings: &SettingsVm, providers: &[ProviderVm]) {
        self.update(|bridge| {
            bridge.set_remote_available(providers.iter().any(|p| p.id == "ytmusic" && p.available))
        });
    }

    fn import_status(&self, pending: bool, message: &str) {
        self.update(|bridge| {
            bridge.set_import_pending(pending);
            bridge.set_import_message(message.into());
        });
    }
    fn set_loading(&self, loading: bool) {
        self.update(|bridge| bridge.set_library_loading(loading));
    }

    fn set_queue(&self, queue: &QueueVm) {
        self.update(|bridge| bridge.set_queue(tracks(&queue.tracks)));
    }

    fn set_queue_ids(&self, ids: &[kazzu_core::QueueItemId]) {
        self.update(|bridge| {
            bridge.set_queue_ids(ModelRc::new(VecModel::from(
                ids.iter()
                    .map(|id| slint::SharedString::from(id.0.to_string()))
                    .collect::<Vec<_>>(),
            )));
        });
    }

    fn set_search(&self, search: &SearchVm) {
        self.update(|bridge| {
            bridge.set_searching(search.state == SearchStateVm::Loading);
            bridge.set_search_message(if search.error_text.is_empty() {
                search.empty_text.as_str().into()
            } else {
                search.error_text.as_str().into()
            });
            let albums = search.albums.iter().map(|item| Tile {
                id: item.id.as_str().into(),
                kind: "album".into(),
                title: item.title.as_str().into(),
                sub: item.artist.as_str().into(),
                cover: image(item.cover.as_ref()),
                ..Default::default()
            });
            let artists = search.artists.iter().map(|item| Tile {
                id: item.id.as_str().into(),
                kind: "artist".into(),
                title: item.name.as_str().into(),
                sub: item.subtitle.as_str().into(),
                cover: image(item.cover.as_ref()),
                ..Default::default()
            });
            bridge.set_search_collections(tiles(
                albums
                    .chain(artists)
                    .chain(search.playlists.iter().map(highlight)),
            ));
            bridge.set_results(tracks(&search.songs));
            bridge.set_recent_searches(ModelRc::new(VecModel::from(
                search
                    .recent
                    .iter()
                    .map(|s| slint::SharedString::from(s.as_str()))
                    .collect::<Vec<_>>(),
            )));
        });
    }

    fn set_detail(&self, detail: &DetailVm) {
        self.update(|bridge| {
            bridge.set_detail_title(detail.title.as_str().into());
            bridge.set_detail_tracks(tracks(&detail.tracks));
            bridge.set_detail_albums(tiles(detail.albums.iter().map(|album| Tile {
                id: album.id.as_str().into(),
                kind: "album".into(),
                title: album.title.as_str().into(),
                sub: album.artist.as_str().into(),
                cover: image(album.cover.as_ref()),
                ..Default::default()
            })));
            bridge.set_editable_playlist(!detail.editable_playlist.is_empty());
            bridge.set_cached_detail(detail.cached_audio);
            bridge.set_detail_message(
                match detail.state {
                    LoadStateVm::Loading => "Carregando…",
                    LoadStateVm::Error | LoadStateVm::Offline => &detail.error_text,
                    LoadStateVm::Empty => &detail.empty_text,
                    LoadStateVm::Ready => "",
                }
                .into(),
            );
        });
    }

    fn open_detail(&self, open: bool) {
        self.update(|bridge| bridge.set_detail_open(open));
    }
    fn set_scan(&self, scan: Option<&ScanVm>) {
        self.update(|bridge| {
            bridge.set_scan_label(scan.map(|s| s.label.as_str()).unwrap_or_default().into())
        });
    }

    fn toast(&self, text: &str, _error: bool, action: &str) {
        self.update(|bridge| {
            bridge.set_notice(text.into());
            bridge.set_notice_action(action.into());
            bridge.set_notice_id(bridge.get_notice_id().wrapping_add(1));
        });
    }

    fn mark_playing(&self, _id: Option<&str>) {}

    fn mark_liked(&self, id: &str, liked: bool) {
        self.update(|bridge| {
            if bridge.get_now().id.as_str() == id {
                bridge.set_liked(liked);
            }
        });
    }

    fn set_prefs(&self, prefs: &PrefsVm) {
        self.update(|bridge| {
            bridge.set_cover_theme(prefs.accent == crate::config::AccentPref::Cover);
            bridge.set_stream_quality(prefs.stream_quality.index());
            bridge.set_autoplay(prefs.autoplay_radio);
            bridge.set_dark(prefs.theme != ThemePref::Light);
            bridge.set_reduce_motion(prefs.reduce_motion);
            bridge.set_reduce_transparency(prefs.reduce_transparency);
        });
    }

    fn set_mobile_appearance(&self, appearance: &crate::config::MobileAppearance) {
        self.update(|bridge| {
            bridge.set_glass(ModelRc::new(VecModel::from(appearance.glass.to_vec())));
            bridge.set_rounded(appearance.rounded);
            bridge.set_effects(appearance.effects);
            bridge.set_vinyl(appearance.vinyl);
            bridge.set_crossfade_seconds(appearance.crossfade_seconds);
        });
    }

    fn set_volume(&self, volume: f32) {
        self.update(|bridge| bridge.set_volume(volume));
    }

    fn sleep_timer_elapsed(&self) {
        self.update(|bridge| bridge.set_sleep_minutes(0));
    }

    fn set_storage(&self, files: u64, cache: u64) {
        self.update(|bridge| {
            let size = |bytes| -> slint::SharedString {
                format!("{:.1} MiB", bytes as f64 / 1_048_576.0).into()
            };
            bridge.set_persistent_usage(size(files));
            bridge.set_cache_usage(size(cache));
        });
    }

    fn set_online(&self, _online: bool) {}
    fn set_perf_offer(&self, _offer: bool) {}
    fn close_sheets(&self) {}
}
