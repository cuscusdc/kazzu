//! View-models: dados prontos para a UI, sem tipos do Slint (podem cruzar threads).
//! A conversão para `slint::Image`/structs do Slint acontece só na thread de UI (`slint_view`).
//!
//! Capas de provedores remotos chegam depois do texto: cada modelo guarda `cover_url` e o controller preenche
//! `cover` quando a imagem é baixada (ver [`CoverSlots`]).

use crate::accent::Accent;
use crate::art::Pixels;
use crate::config::{AccentPref, StreamQualityPref, ThemePref};

#[derive(Clone, Default)]
pub struct TrackVm {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub cover: Option<Pixels>,
    /// Capa remota ainda não baixada (ou já baixada, se `cover` existe).
    pub cover_url: Option<String>,
    pub duration_label: String,
    pub codec: String,
    pub detail: String,
    pub detail_long: String,
    pub hires: bool,
    /// "" = local; "YouTube Music", "Spotify"...
    pub source: String,
    pub liked: bool,
    pub playing: bool,
    /// Motivo da sugestão ("Em alta hoje"); vazio = nenhum.
    pub why: String,
}

#[derive(Clone, Default)]
pub struct AlbumVm {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub cover: Option<Pixels>,
    pub cover_url: Option<String>,
}

#[derive(Clone, Default)]
pub struct HighlightVm {
    pub id: String,
    /// "album" | "playlist" | "artist" | "mood" | "shuffle" | "liked" | "track".
    pub kind: String,
    pub caption: String,
    pub title: String,
    pub subtitle: String,
    pub cover: Option<Pixels>,
    pub cover_url: Option<String>,
    /// Cor da placa quando não há capa (humores do YouTube Music).
    pub tint: Option<[u8; 3]>,
}

#[derive(Clone, Default)]
pub struct ArtistVm {
    pub id: String,
    pub name: String,
    pub subtitle: String,
    pub cover: Option<Pixels>,
    pub cover_url: Option<String>,
}

#[derive(Clone, Default)]
pub struct PlaylistVm {
    pub id: String,
    pub name: String,
    pub count_label: String,
    pub covers: Vec<Pixels>,
}

#[derive(Clone, Default)]
pub struct CollectionVm {
    /// "downloads" | "liked" | "albums" | "playlists"
    pub id: String,
    pub title: String,
    pub count_label: String,
    pub covers: Vec<Pixels>,
    /// Cartão "Em breve".
    pub soon: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum LoadStateVm {
    #[default]
    Loading,
    Ready,
    Empty,
    Error,
    Offline,
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, Hash)]
pub enum HomeSectionVm {
    #[default]
    ForYou,
    Discover,
    Mixes,
    Focus,
    Workout,
}

#[derive(Clone, Default)]
pub struct HomeVm {
    pub state: LoadStateVm,
    pub section: HomeSectionVm,
    /// Erro, mas há conteúdo em cache na tela.
    pub stale: bool,
    pub error_title: String,
    pub error_text: String,
    pub error_code: String,
    pub source_label: String,
    /// Aviso de primeiro uso ainda não dispensado.
    pub banner: bool,
    pub highlights: Vec<HighlightVm>,
    pub quick_picks: Vec<TrackVm>,
    pub mixes: Vec<HighlightVm>,
    pub albums: Vec<AlbumVm>,
    /// Faixas locais recentes (Início offline, Biblioteca).
    pub recents: Vec<TrackVm>,
}

#[derive(Clone, Default)]
pub struct ExploreVm {
    pub state: LoadStateVm,
    pub error_text: String,
    pub new_releases: Vec<AlbumVm>,
    pub moods: Vec<HighlightVm>,
    pub trending: Vec<TrackVm>,
}

#[derive(Clone, Default)]
pub struct LibraryVm {
    #[cfg(target_os = "android")]
    pub genres: Vec<String>,
    pub listening_summary: String,
    pub albums: Vec<AlbumVm>,
    pub artists: Vec<ArtistVm>,
    pub collections: Vec<CollectionVm>,
    pub playlists: Vec<PlaylistVm>,
}

#[derive(Clone, Default)]
pub struct SettingsVm {
    pub folders: Vec<String>,
    pub summary: String,
    pub has_library: bool,
    pub track_count: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum DetailKindVm {
    #[default]
    Tracks,
    Albums,
    Artist,
}

#[derive(Clone, Default)]
pub struct DetailVm {
    #[cfg(target_os = "android")]
    pub cached_audio: bool,
    pub editable_playlist: String,
    pub description: String,
    pub related: Vec<HighlightVm>,
    pub kind: DetailKindVm,
    pub state: LoadStateVm,
    pub title: String,
    pub subtitle: String,
    pub cover: Option<Pixels>,
    pub cover_url: Option<String>,
    /// "Do YouTube Music" ou vazio.
    pub source: String,
    pub empty_text: String,
    pub error_text: String,
    pub saved: bool,
    pub can_save: bool,
    pub tracks: Vec<TrackVm>,
    pub albums: Vec<AlbumVm>,
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum RepeatVm {
    #[default]
    Off,
    All,
    One,
}

#[derive(Clone, Default)]
pub struct PlaybackVm {
    pub has_now: bool,
    pub now: Option<TrackVm>,
    pub playing: bool,
    pub loading: bool,
    pub volume: f32,
    pub shuffle: bool,
    pub repeat: RepeatVm,
    pub output_name: String,
    pub output_note: String,
    pub bit_perfect: bool,
    /// "Mix Foco" (de onde veio a fila).
    pub origin: String,
    /// "Do YouTube Music" | "Neste aparelho".
    pub source: String,
    /// "OPUS 160 kbps".
    pub quality_label: String,
    pub error: String,
    pub error_action: String,
}

/// Sem capa o console fica sólido (SLINT_SPEC v4 §6).
pub const PLAYER_DIM_NO_COVER: f32 = 0.64;
pub const MINI_DIM_NO_COVER: f32 = 0.70;

#[derive(Clone)]
pub struct NowArtVm {
    /// Capa grande (≤ 640 px) do player.
    pub cover: Option<Pixels>,
    /// Miniatura (≤ 192 px) do mini player, da NowBar e da fila.
    pub thumb: Option<Pixels>,
    /// 192x464: capa, reflexo e topo da capa (atrás do console do player).
    pub backdrop: Option<Pixels>,
    /// 96x96 muito desfocada: tinge o mini-player e fica atrás do player em janelas largas.
    pub backdrop_wide: Option<Pixels>,
    /// Escurecimento do console do player (0,24 a 0,64) e do mini-player (0,30 a 0,70).
    pub player_dim: f32,
    pub mini_dim: f32,
    pub accent: Accent,
}

impl Default for NowArtVm {
    fn default() -> Self {
        Self { cover: None, thumb: None, backdrop: None, backdrop_wide: None, player_dim: PLAYER_DIM_NO_COVER, mini_dim: MINI_DIM_NO_COVER, accent: Accent::BRAND }
    }
}

#[derive(Clone, Default)]
pub struct QueueVm {
    pub tracks: Vec<TrackVm>,
    pub summary: String,
    /// "e mais 1.900 músicas" quando a fila é maior que a parte mostrada.
    pub more_label: String,
    /// "Quando a fila acabar, o Kazzu continua com…"; vazio = sem nota.
    pub note: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum SearchStateVm {
    #[default]
    Idle,
    Results,
    Empty,
    Loading,
    Error,
    Offline,
}

#[derive(Clone, Default)]
pub struct SearchVm {
    pub state: SearchStateVm,
    pub empty_text: String,
    pub error_text: String,
    pub recent: Vec<String>,
    pub suggestions: Vec<String>,
    pub best: Option<HighlightVm>,
    pub songs: Vec<TrackVm>,
    pub albums: Vec<AlbumVm>,
    pub artists: Vec<ArtistVm>,
    pub playlists: Vec<HighlightVm>,
}

#[derive(Clone, Default)]
pub struct ScanVm {
    /// 0..1, ou negativo = indeterminado.
    pub progress: f32,
    pub label: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PrefsVm {
    pub theme: ThemePref,
    pub accent: AccentPref,
    pub reduce_motion: bool,
    pub reduce_transparency: bool,
    pub stream_quality: StreamQualityPref,
    pub autoplay_radio: bool,
}

#[derive(Clone, Default)]
pub struct ProviderVm {
    pub id: String,
    pub name: String,
    pub status_label: String,
    pub connected: bool,
    pub available: bool,
}

// ---------------------------------------------------------------------------------------------
// Capas que chegam depois
// ---------------------------------------------------------------------------------------------

/// Um modelo que expõe suas capas remotas: o controller as preenche quando o download termina.
pub trait CoverSlots {
    /// Chama `f(url, slot)` para cada capa do modelo.
    fn each_cover(&mut self, f: &mut dyn FnMut(Option<&str>, &mut Option<Pixels>));
}

impl CoverSlots for TrackVm {
    fn each_cover(&mut self, f: &mut dyn FnMut(Option<&str>, &mut Option<Pixels>)) {
        f(self.cover_url.as_deref(), &mut self.cover);
    }
}

impl CoverSlots for AlbumVm {
    fn each_cover(&mut self, f: &mut dyn FnMut(Option<&str>, &mut Option<Pixels>)) {
        f(self.cover_url.as_deref(), &mut self.cover);
    }
}

impl CoverSlots for HighlightVm {
    fn each_cover(&mut self, f: &mut dyn FnMut(Option<&str>, &mut Option<Pixels>)) {
        f(self.cover_url.as_deref(), &mut self.cover);
    }
}

impl CoverSlots for ArtistVm {
    fn each_cover(&mut self, f: &mut dyn FnMut(Option<&str>, &mut Option<Pixels>)) {
        f(self.cover_url.as_deref(), &mut self.cover);
    }
}

fn each_in<T: CoverSlots>(list: &mut [T], f: &mut dyn FnMut(Option<&str>, &mut Option<Pixels>)) {
    for item in list {
        item.each_cover(f);
    }
}

impl CoverSlots for HomeVm {
    fn each_cover(&mut self, f: &mut dyn FnMut(Option<&str>, &mut Option<Pixels>)) {
        each_in(&mut self.highlights, f);
        each_in(&mut self.quick_picks, f);
        each_in(&mut self.mixes, f);
        each_in(&mut self.albums, f);
    }
}

impl CoverSlots for ExploreVm {
    fn each_cover(&mut self, f: &mut dyn FnMut(Option<&str>, &mut Option<Pixels>)) {
        each_in(&mut self.new_releases, f);
        each_in(&mut self.moods, f);
        each_in(&mut self.trending, f);
    }
}

impl CoverSlots for SearchVm {
    fn each_cover(&mut self, f: &mut dyn FnMut(Option<&str>, &mut Option<Pixels>)) {
        if let Some(b) = self.best.as_mut() {
            b.each_cover(f);
        }
        each_in(&mut self.songs, f);
        each_in(&mut self.albums, f);
        each_in(&mut self.artists, f);
        each_in(&mut self.playlists, f);
    }
}

impl CoverSlots for DetailVm {
    fn each_cover(&mut self, f: &mut dyn FnMut(Option<&str>, &mut Option<Pixels>)) {
        each_in(&mut self.related, f);
        f(self.cover_url.as_deref(), &mut self.cover);
        each_in(&mut self.tracks, f);
        each_in(&mut self.albums, f);
    }
}

impl CoverSlots for QueueVm {
    fn each_cover(&mut self, f: &mut dyn FnMut(Option<&str>, &mut Option<Pixels>)) {
        each_in(&mut self.tracks, f);
    }
}

/// URLs de capa que ainda faltam baixar em `vm`, sem repetição, na ordem em que aparecem.
pub fn missing_covers(vm: &mut dyn CoverSlots) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    vm.each_cover(&mut |url, slot| {
        if let Some(u) = url
            && slot.is_none()
            && !out.iter().any(|o| o == u)
        {
            out.push(u.to_owned());
        }
    });
    out
}

/// Preenche as capas de `vm` que `lookup` conhecer. Devolve quantas foram preenchidas.
pub fn apply_covers(vm: &mut dyn CoverSlots, lookup: &mut dyn FnMut(&str) -> Option<Pixels>) -> usize {
    let mut n = 0;
    vm.each_cover(&mut |url, slot| {
        if slot.is_none()
            && let Some(u) = url
            && let Some(px) = lookup(u)
        {
            *slot = Some(px);
            n += 1;
        }
    });
    n
}


pub struct FolderListingVm {
    pub current: String,
    pub parent: String,
    pub children: Vec<(String, String)>,
}
