//! Trabalho de rede dos provedores remotos, fora da thread de UI.
//!
//! [`RemoteWorker::run`] executa um [`RemoteJob`] e entrega [`RemoteResult`]s (um job pode entregar mais de um:
//! o Início manda primeiro o cache e depois o feed novo). É síncrono e sem threads próprias: o app o chama de
//! `ThreadedRunner` (uma thread por job, com debounce da busca) e os testes o chamam na hora, com um `Catalog` falso.

use std::collections::HashSet;
use std::sync::Arc;

use kazzu_core::{Album, ProviderKind, QueueItem, QueueItemId, StreamQuality, StreamSource, Track};
use kazzu_ytm::{BrowseRef, Category, Item, Radio, SearchFilter};

use crate::art::{self, Pixels};
use crate::catalog::{Catalog, ErrorKind, ExploreFeed, Providers, YtmError};
use crate::covers::{CoverFetcher, PLAYER_SIDE};
use crate::vm::*;
use crate::ytm_vm::{self, remote_id};
use crate::{accent, worker::JobResult};

/// O que abrir na tela de detalhe ou tocar de uma vez.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteRef {
    Album(String),
    Playlist(String),
    Artist(String),
    /// Humor ou gênero (Explorar): título + destino.
    Browse { title: String, target: BrowseRef },
}

impl RemoteRef {
    /// Álbuns do YouTube Music têm id `MPREb_…` (ou a playlist de áudio `OLAK5uy_…`); o resto é playlist.
    pub fn album_or_playlist(id: &str) -> Self {
        if id.starts_with("MPRE") || id.starts_with("OLAK5") { Self::Album(id.to_owned()) } else { Self::Playlist(id.to_owned()) }
    }
}

#[derive(Debug, Clone)]
pub enum RemoteJob {
    LoadHome { seq: u64, section: HomeSectionVm },
    LoadExplore { seq: u64 },
    /// `kind`: 0 tudo, 1 músicas, 2 álbuns, 3 artistas, 4 playlists.
    Search { seq: u64, query: String, kind: i32 },
    Suggest { seq: u64, prefix: String },
    Open { seq: u64, what: RemoteRef },
    /// Busca as faixas de um álbum/playlist/artista para tocar agora.
    PlayItem { ctx_seq: u64, what: RemoteRef, shuffle: bool },
    /// Abre uma faixa para o motor. `for_start`: é a que o usuário pediu (toca já); senão, entra no fim da fila.
    Resolve { ctx_seq: u64, track: Track, quality: StreamQuality, for_start: bool },
    /// Abre uma faixa para "Tocar em seguida" / "Adicionar à fila".
    ResolveQueued { track: Track, quality: StreamQuality, play_next: bool },
    /// URL vencida no meio da faixa: esquece a antiga e resolve de novo.
    Reresolve { id: QueueItemId, track: Track, quality: StreamQuality },
    Radio { ctx_seq: u64, seed: String, prior: Option<Box<Radio>> },
    Covers { urls: Vec<String> },
    NowArt { seq: u64, url: String },
    #[cfg(target_os = "android")]
    Lyrics { seq: u64, id: String },
    Like { track: Track, liked: bool },
    SaveAlbum { album: Album, saved: bool },
}

pub enum RemoteResult {
    /// `cached`: veio do cache em disco (sem rede); o feed novo ainda pode chegar.
    Home { seq: u64, vm: Box<HomeVm>, tracks: Vec<Track>, cached: bool },
    HomeFailed { seq: u64, kind: ErrorKind, message: String },
    Explore { seq: u64, vm: Box<ExploreVm>, tracks: Vec<Track>, categories: Vec<Category> },
    ExploreFailed { seq: u64, kind: ErrorKind },
    Search { seq: u64, kind: i32, vm: Box<SearchVm>, tracks: Vec<Track> },
    SearchFailed { seq: u64, kind: ErrorKind },
    Suggestions { seq: u64, list: Vec<String> },
    Detail { seq: u64, vm: Box<DetailVm>, tracks: Vec<Track>, what: RemoteRef, album: Option<Album> },
    DetailFailed { seq: u64, kind: ErrorKind },
    PlayTracks { ctx_seq: u64, tracks: Vec<Track>, label: String, shuffle: bool },
    PlayFailed { ctx_seq: u64, kind: ErrorKind },
    Resolved { ctx_seq: u64, item: Box<QueueItem>, quality_label: String, for_start: bool },
    ResolveFailed { ctx_seq: u64, track: Box<Track>, kind: ErrorKind, for_start: bool },
    Queued { item: Box<QueueItem>, quality_label: String, play_next: bool },
    QueueFailed { kind: ErrorKind },
    Replaced { id: QueueItemId, source: StreamSource, quality_label: String },
    ReplaceFailed { id: QueueItemId, kind: ErrorKind },
    Radio { ctx_seq: u64, tracks: Vec<Track>, radio: Box<Radio> },
    RadioFailed { ctx_seq: u64, kind: ErrorKind },
    Covers(Vec<(String, Pixels)>),
}

pub type Emit<'a> = &'a (dyn Fn(JobResult) + Sync);

/// Rede de provedores + capas. Barato de compartilhar (`Arc`).
pub struct RemoteWorker {
    providers: Providers,
    covers: CoverFetcher,
}

fn search_filter(kind: i32) -> Option<SearchFilter> {
    match kind {
        1 => Some(SearchFilter::Songs),
        2 => Some(SearchFilter::Albums),
        3 => Some(SearchFilter::Artists),
        4 => Some(SearchFilter::FeaturedPlaylists),
        _ => None,
    }
}

/// Consulta temática dos chips Foco e Treino, no idioma do sistema (nunca fixa em inglês).
pub fn theme_query(section: HomeSectionVm, lang: &str) -> Option<&'static str> {
    let focus = matches!(section, HomeSectionVm::Focus);
    let workout = matches!(section, HomeSectionVm::Workout);
    if !focus && !workout {
        return None;
    }
    Some(match (lang, focus) {
        ("pt", true) => "música para foco",
        ("pt", false) => "música para treino",
        ("es", true) => "música para concentrarse",
        ("es", false) => "música para entrenar",
        ("fr", true) => "musique pour se concentrer",
        ("fr", false) => "musique pour le sport",
        ("de", true) => "Musik zum Konzentrieren",
        ("de", false) => "Workout Musik",
        (_, true) => "focus music",
        (_, false) => "workout music",
    })
}

fn system_language() -> String {
    sys_locale::get_locale().and_then(|l| l.split(['-', '_']).next().map(str::to_ascii_lowercase)).unwrap_or_default()
}

/// Cartões "Mix" / playlists da Mixes: só playlists de verdade (sem faixas soltas).
fn liked_set() -> HashSet<String> {
    HashSet::new()
}

impl RemoteWorker {
    pub fn new(providers: Providers, covers: CoverFetcher) -> Arc<Self> {
        Arc::new(Self { providers, covers })
    }

    pub fn providers(&self) -> &Providers {
        &self.providers
    }

    fn catalog(&self) -> Option<&Arc<dyn Catalog>> {
        self.providers.default_remote()
    }

    pub fn run(&self, job: RemoteJob, emit: Emit<'_>) {
        let Some(cat) = self.catalog() else {
            emit(JobResult::Failed("Nenhum provedor de música está ativo.".into()));
            return;
        };
        let out = |r: RemoteResult| emit(JobResult::Remote(r));
        match job {
            RemoteJob::LoadHome { seq, section } => self.load_home(cat.as_ref(), seq, section, &out),
            RemoteJob::LoadExplore { seq } => match cat.explore() {
                Ok(feed) => out(self.explore_result(seq, &feed)),
                Err(e) => out(RemoteResult::ExploreFailed { seq, kind: e.kind() }),
            },
            RemoteJob::Search { seq, query, kind } => match cat.search(&query, search_filter(kind), 30) {
                Ok(items) => {
                    let mut tracks = Vec::new();
                    let vm = ytm_vm::build_search(&items, kind, &liked_set(), &mut tracks);
                    out(RemoteResult::Search { seq, kind, vm: Box::new(vm), tracks });
                }
                Err(e) => out(RemoteResult::SearchFailed { seq, kind: e.kind() }),
            },
            RemoteJob::Suggest { seq, prefix } => {
                // sugestões são opcionais: falhar não é erro para o usuário
                if let Ok(s) = cat.suggestions(&prefix) {
                    out(RemoteResult::Suggestions { seq, list: s.queries.into_iter().take(6).collect() });
                }
            }
            RemoteJob::Open { seq, what } => match self.open(cat.as_ref(), &what) {
                Ok((vm, tracks, album)) => out(RemoteResult::Detail { seq, vm: Box::new(vm), tracks, what, album }),
                Err(e) => out(RemoteResult::DetailFailed { seq, kind: e.kind() }),
            },
            RemoteJob::PlayItem { ctx_seq, what, shuffle } => match self.item_tracks(cat.as_ref(), &what) {
                Ok((label, tracks)) if !tracks.is_empty() => out(RemoteResult::PlayTracks { ctx_seq, tracks, label, shuffle }),
                Ok(_) => out(RemoteResult::PlayFailed { ctx_seq, kind: ErrorKind::NotFound }),
                Err(e) => out(RemoteResult::PlayFailed { ctx_seq, kind: e.kind() }),
            },
            RemoteJob::Resolve { ctx_seq, track, quality, for_start } => match cat.resolve(&track, quality) {
                Ok(r) => out(RemoteResult::Resolved { ctx_seq, item: Box::new(QueueItem::new(r.track, r.source)), quality_label: r.quality_label, for_start }),
                Err(e) => out(RemoteResult::ResolveFailed { ctx_seq, track: Box::new(track), kind: e.kind(), for_start }),
            },
            RemoteJob::ResolveQueued { track, quality, play_next } => match cat.resolve(&track, quality) {
                Ok(r) => out(RemoteResult::Queued { item: Box::new(QueueItem::new(r.track, r.source)), quality_label: r.quality_label, play_next }),
                Err(e) => out(RemoteResult::QueueFailed { kind: e.kind() }),
            },
            RemoteJob::Reresolve { id, track, quality } => {
                cat.invalidate(&track);
                match cat.resolve(&track, quality) {
                    Ok(r) => out(RemoteResult::Replaced { id, source: r.source, quality_label: r.quality_label }),
                    Err(e) => out(RemoteResult::ReplaceFailed { id, kind: e.kind() }),
                }
            }
            RemoteJob::Radio { ctx_seq, seed, prior } => {
                let result = match prior.as_deref().filter(|r| r.continuation.is_some()) {
                    Some(r) => cat.radio_continuation(r),
                    None => cat.radio(&seed),
                };
                match result {
                    Ok(radio) => out(RemoteResult::Radio { ctx_seq, tracks: radio.tracks.clone(), radio: Box::new(radio) }),
                    Err(e) => out(RemoteResult::RadioFailed { ctx_seq, kind: e.kind() }),
                }
            }
            RemoteJob::Covers { urls } => {
                // entrega em lotes pequenos: a tela ganha capas aos poucos sem republicar a cada imagem
                let batch = std::sync::Mutex::new(Vec::new());
                self.covers.fetch_many(&urls, &|url, px| {
                    let mut b = batch.lock().unwrap_or_else(|p| p.into_inner());
                    b.push((url, px));
                    if b.len() >= 4 {
                        out(RemoteResult::Covers(std::mem::take(&mut *b)));
                    }
                });
                let rest = batch.into_inner().unwrap_or_else(|p| p.into_inner());
                if !rest.is_empty() {
                    out(RemoteResult::Covers(rest));
                }
            }
            #[cfg(target_os = "android")]
            RemoteJob::Lyrics { seq, id } => {
                emit(JobResult::RemoteLyrics { seq, result: cat.lyrics(&id).map_err(|error| error.kind()) });
            }
            RemoteJob::NowArt { seq, url } => {
                let art = match self.covers.image(&url, PLAYER_SIDE) {
                    Some(img) => {
                        let (player_dim, mini_dim) = art::glass_dims(&img);
                        NowArtVm {
                            accent: accent::derive(&img),
                            backdrop: Some(art::to_pixels(&art::backdrop(&img))),
                            backdrop_wide: Some(art::to_pixels(&art::backdrop_wide(&img))),
                            player_dim,
                            mini_dim,
                            thumb: Some(art::to_pixels(&art::fit_within(img.clone(), 192))),
                            cover: Some(art::to_pixels(&img)),
                        }
                    }
                    None => NowArtVm::default(),
                };
                emit(JobResult::NowArt { seq, art: Box::new(art) });
            }
            RemoteJob::Like { track, liked } => {
                let id = remote_id(track.id.as_str());
                let res = if liked { cat.like(&track) } else { cat.unlike(&track.id).map(|_| ()) };
                match res {
                    Ok(()) => emit(JobResult::LikedSaved { id, liked }),
                    Err(e) => emit(JobResult::Failed(format!("Não deu para guardar a curtida. {e}"))),
                }
            }
            RemoteJob::SaveAlbum { album, saved } => {
                let res = if saved { cat.save_album(&album) } else { cat.remove_album(album.id.as_str()).map(|_| ()) };
                if let Err(e) = res {
                    emit(JobResult::Failed(format!("Não deu para guardar o álbum. {e}")));
                }
            }
        }
    }

    fn load_home(&self, cat: &dyn Catalog, seq: u64, section: HomeSectionVm, out: &dyn Fn(RemoteResult)) {
        let liked = liked_set();
        if let Some(query) = theme_query(section, &system_language()) {
            match cat.search(query, Some(SearchFilter::FeaturedPlaylists), 20) {
                Ok(items) => {
                    let mut tracks = Vec::new();
                    let vm = ytm_vm::build_home_from_items(&items, section, &liked, &mut tracks);
                    out(RemoteResult::Home { seq, vm: Box::new(vm), tracks, cached: false });
                }
                Err(e) => out(RemoteResult::HomeFailed { seq, kind: e.kind(), message: e.to_string() }),
            }
            return;
        }
        // 1) cache em disco: o conteúdo aparece sem esperar a rede
        if section == HomeSectionVm::ForYou
            && let Some(shelves) = cat.peek_home()
        {
            let mut tracks = Vec::new();
            let vm = ytm_vm::build_home(&shelves, section, &liked, &mut tracks);
            if vm.state == LoadStateVm::Ready {
                out(RemoteResult::Home { seq, vm: Box::new(vm), tracks, cached: true });
            }
        }
        // 2) feed novo (home + lançamentos + paradas do país)
        match cat.discover() {
            Ok(d) => {
                let mut tracks = Vec::new();
                let vm = ytm_vm::build_home(&d.shelves, section, &liked, &mut tracks);
                out(RemoteResult::Home { seq, vm: Box::new(vm), tracks, cached: false });
            }
            Err(e) => out(RemoteResult::HomeFailed { seq, kind: e.kind(), message: e.to_string() }),
        }
    }

    fn explore_result(&self, seq: u64, feed: &ExploreFeed) -> RemoteResult {
        let mut tracks = Vec::new();
        let vm = ytm_vm::build_explore(feed, &liked_set(), &mut tracks);
        RemoteResult::Explore { seq, vm: Box::new(vm), tracks, categories: feed.categories.clone() }
    }

    fn open(&self, cat: &dyn Catalog, what: &RemoteRef) -> Result<(DetailVm, Vec<Track>, Option<Album>), YtmError> {
        let mut tracks = Vec::new();
        let liked = liked_set();
        Ok(match what {
            RemoteRef::Album(id) => {
                let page = cat.album(id)?;
                let saved = cat.saved_albums().iter().any(|a| a.id.as_str() == id);
                let vm = ytm_vm::build_album(&page, &liked, saved, &mut tracks);
                (vm, tracks, Some(page.album))
            }
            RemoteRef::Playlist(id) => {
                let page = cat.playlist(id)?;
                (ytm_vm::build_playlist(&page, &liked, &mut tracks), tracks, None)
            }
            RemoteRef::Artist(id) => {
                let page = cat.artist(id)?;
                (ytm_vm::build_artist(&page, &liked, &mut tracks), tracks, None)
            }
            RemoteRef::Browse { title, target } => {
                let page = cat.browse(target)?;
                let mut vm = DetailVm { kind: DetailKindVm::Artist, title: title.clone(), source: ytm_vm::SOURCE_LABEL.to_owned(), empty_text: "Nada por aqui ainda.".into(), ..Default::default() };
                for item in page.shelves.iter().flat_map(|s| s.items.iter()) {
                    match item {
                        Item::Track(t) if vm.tracks.len() < 30 => {
                            vm.tracks.push(ytm_vm::track_vm(t, false, ""));
                            tracks.push(t.clone());
                        }
                        Item::Album(a) if vm.albums.len() < 24 => vm.albums.push(ytm_vm::album_vm(a)),
                        Item::Playlist(p) if vm.albums.len() < 24 => vm.albums.push(AlbumVm { id: remote_id(p.id.as_str()), title: p.name.clone(), artist: "Playlist".into(), cover: None, cover_url: ytm_vm::highlight(item, "").cover_url }),
                        _ => {}
                    }
                }
                vm.state = if vm.tracks.is_empty() && vm.albums.is_empty() { LoadStateVm::Empty } else { LoadStateVm::Ready };
                (vm, tracks, None)
            }
        })
    }

    fn item_tracks(&self, cat: &dyn Catalog, what: &RemoteRef) -> Result<(String, Vec<Track>), YtmError> {
        Ok(match what {
            RemoteRef::Album(id) => {
                let p = cat.album(id)?;
                (p.album.title, p.tracks)
            }
            RemoteRef::Playlist(id) => {
                let p = cat.playlist(id)?;
                (p.playlist.name, p.tracks)
            }
            RemoteRef::Artist(id) => {
                let p = cat.artist(id)?;
                (p.artist.name, p.top_tracks)
            }
            RemoteRef::Browse { title, target } => {
                let page = cat.browse(target)?;
                let tracks = page.shelves.iter().flat_map(|s| s.items.iter()).filter_map(|i| if let Item::Track(t) = i { Some(t.clone()) } else { None }).collect();
                (title.clone(), tracks)
            }
        })
    }
}

/// O provedor de uma faixa (para escolher o catálogo certo quando houver mais de um).
pub fn provider_of(t: &Track) -> ProviderKind {
    t.provider()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_queries_follow_the_system_language() {
        assert_eq!(theme_query(HomeSectionVm::Focus, "pt"), Some("música para foco"));
        assert_eq!(theme_query(HomeSectionVm::Workout, "pt"), Some("música para treino"));
        assert_eq!(theme_query(HomeSectionVm::Focus, "ja"), Some("focus music"));
        assert_eq!(theme_query(HomeSectionVm::ForYou, "pt"), None);
    }

    #[test]
    fn ids_choose_album_or_playlist() {
        assert_eq!(RemoteRef::album_or_playlist("MPREb_x"), RemoteRef::Album("MPREb_x".into()));
        assert_eq!(RemoteRef::album_or_playlist("OLAK5uy_x"), RemoteRef::Album("OLAK5uy_x".into()));
        assert_eq!(RemoteRef::album_or_playlist("RDCLAK5uy_x"), RemoteRef::Playlist("RDCLAK5uy_x".into()));
    }
}
