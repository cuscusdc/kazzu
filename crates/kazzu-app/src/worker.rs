//! Trabalho pesado fora da thread de UI: consultas à biblioteca, scan, decodificação de capas.
//!
//! `Store` é síncrono e testável (`Store::run`). O app usa `ThreadedRunner`, que roda um `Store` numa thread e o
//! scan numa thread própria (com outra conexão SQLite; o banco está em modo WAL).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use std::time::Instant;
use kazzu_core::QueueItem;
use kazzu_library::{Album, AlbumId, ArtistId, Artwork, Library, PlaylistEntry, PlaylistId, ScanProgress, ScanReport, Track, TrackId};

use crate::accent;
use crate::art::{self, Pixels};
use crate::catalog::Providers;
use crate::format;
use crate::lru::Lru;
use crate::remote::{RemoteJob, RemoteResult, RemoteWorker};
use crate::vm::*;
use crate::ytm_vm;

/// Espera após a última digitação antes de consultar (debounce da busca).
pub const SEARCH_DEBOUNCE: Duration = Duration::from_millis(200);
/// Debounce das consultas ao provedor remoto (busca e sugestões): menos rede por tecla.
pub const REMOTE_DEBOUNCE: Duration = Duration::from_millis(250);
const SEARCH_LIMIT: usize = 50;
const RECENTS: usize = 20;
const HOME_ALBUMS: usize = 24;
const THUMB_CACHE_MAX: usize = if cfg!(target_os = "android") { 144 } else { 400 };
const MOBILE_ALBUM_THUMBNAILS: usize = 48;

/// O que tocar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    Album(String),
    Liked,
    Playlist(String),
    Genre(String),
    SmartRecent,
    SmartUnplayed,
    /// Biblioteca inteira.
    All,
    /// Lista exata de faixas (recentes, resultados de busca, detalhe aberto).
    Ids(Vec<String>),
}

/// O que mostrar na tela de detalhe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetailRef {
    Artist(String),
    ArtistOfTrack(String),
    History,
    Songs,
    Album(String),
    Liked,
    Downloads,
    Albums,
    Playlist(String),
    Genre(String),
    SmartRecent,
    SmartUnplayed,
    FavoriteMix,
    /// O álbum a que a faixa (id local) pertence.
    AlbumOfTrack(String),
}

#[derive(Debug, Clone)]
pub enum PlaylistEdit {
    Rename(String),
    Delete,
    Remove { index: usize, track: String },
    MoveUp { index: usize, track: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueMode {
    PlayNext,
    Enqueue,
}

#[derive(Debug, Clone)]
pub enum Job {
    /// Recarrega tudo que a UI mostra (início, biblioteca, configurações).
    Refresh,
    Scan,
    AddFolder(PathBuf),
    RemoveFolder(PathBuf),
    EditPlaylist { id: String, edit: PlaylistEdit },
    BrowseFolders { seq: u64, path: PathBuf, root: PathBuf },
    ImportAccount { lists: Vec<crate::accounts::Import>, skipped: usize },
    Search { seq: u64, query: String },
    Play { scope: Scope, start_id: Option<String>, shuffle: bool, label: String },
    PlayScoped { seq: u64, scope: Scope, start_id: Option<String>, shuffle: bool, label: String },
    QueueItems { id: String, mode: QueueMode },
    /// Resolve local database IDs and leave remote transport deferred to the engine.
    PrepareQueue { seq: u64, entries: Vec<crate::controller::Entry>, start: usize, shuffle: bool, label: String },
    OpenDetail { seq: u64, what: DetailRef },
    NowArt { seq: u64, cover: Option<PathBuf> },
    #[cfg(target_os = "android")]
    Lyrics { seq: u64, path: PathBuf },
    #[cfg(target_os = "android")]
    ListeningStats { seq: u64, from: i64, until: i64, export: bool },
    #[cfg(target_os = "android")]
    EditLocalTags { root: PathBuf, track_id: String, title: String, artist: String, album: String, genre: String },
    #[cfg(target_os = "android")]
    ImportM3u { title: String, contents: String },
    #[cfg(target_os = "android")]
    ExportBackup { root: PathBuf, config: Box<crate::config::Config>, lan: bool, room: bool },
    #[cfg(target_os = "android")]
    RestoreBackup { root: PathBuf, json: String },
    #[cfg(target_os = "android")]
    SyncPeerPlaylists { root: PathBuf, json: String },
    #[cfg(target_os = "android")]
    SetPlaylistCover { id: String, bytes: Vec<u8> },
    #[cfg(target_os = "android")]
    OpenCachedAudio { seq: u64, cache: Arc<kazzu_audio::StreamCache> },
    /// View-models (com miniaturas) de faixas já conhecidas, para a fila.
    TrackVms { seq: u64, ids: Vec<String> },
    SetLiked { id: String, liked: bool },
    RecordPlay { id: String, listened_ms: u64 },
    RecordRemotePlay { track: Box<kazzu_core::Track>, listened_ms: u64 },
    /// Cria a playlist e, se `add` vier, já coloca a faixa nela (`id`, e a faixa completa se for remota).
    CreatePlaylist { name: String, add: Option<(String, Option<Box<kazzu_core::Track>>)> },
    /// Põe uma faixa (local, ou remota por referência) numa playlist.
    AddToPlaylist { playlist_id: String, track_id: String, remote: Option<Box<kazzu_core::Track>> },
    /// Apaga o histórico de escuta (só local).
    ClearHistory,
    /// Grava a configuração (arquivo TOML, escrita atômica) fora da thread de UI.
    SaveConfig { path: PathBuf, config: Box<crate::config::Config> },
    /// Grava o texto de "Relatar um problema" (só local; nada é enviado).
    WriteReport { path: PathBuf, text: String },
    /// Abre uma faixa LOCAL de uma fila gerenciada pelo controller (ver `RemoteResult::Resolved`).
    ResolveLocal { ctx_seq: u64, id: String, for_start: bool },
    /// Trabalho de rede dos provedores remotos (roda fora da thread da biblioteca).
    Remote(RemoteJob),
}

pub struct Snapshot {
    pub home: HomeVm,
    pub library: LibraryVm,
    pub settings: SettingsVm,
    pub ambient: Option<Pixels>,
    /// Ids curtidos: locais (numéricos) e remotos (`yt:…`).
    pub liked: HashSet<String>,
    /// Faixas remotas curtidas (o controller as guarda para poder tocá-las).
    pub remote_liked: Vec<kazzu_core::Track>,
}

pub enum JobResult {
    DetailFailed { seq: u64, message: String },
    PlaylistEdited { id: String, deleted: bool },
    AccountImported(Result<String, String>),
    Refreshed(Box<Snapshot>),
    ScanProgress(ScanVm),
    ScanDone(Result<ScanReport, String>),
    Search { seq: u64, songs: Vec<TrackVm>, albums: Vec<AlbumVm>, ids: Vec<String> },
    Queue { items: Vec<QueueItem>, start: usize, shuffle: bool, label: String },
    QueueItems { items: Vec<QueueItem>, mode: QueueMode },
    PreparedQueue { seq: u64, items: Vec<QueueItem>, start: usize, shuffle: bool, label: String },
    Detail { seq: u64, detail: Box<DetailVm>, ids: Vec<String>, remote_tracks: Vec<kazzu_core::Track> },
    NowArt { seq: u64, art: Box<NowArtVm> },
    #[cfg(target_os = "android")]
    Lyrics { seq: u64, lines: Vec<(i32, String)> },
    #[cfg(target_os = "android")]
    ListeningStats { seq: u64, stats: Box<kazzu_library::ListeningStats>, export: bool },
    #[cfg(target_os = "android")]
    LocalTagsEdited(Result<(), String>),
    #[cfg(target_os = "android")]
    M3uImported { count: usize, skipped: usize },
    #[cfg(target_os = "android")]
    BackupReady { result: Result<String, String>, lan: bool, room: bool },
    #[cfg(target_os = "android")]
    BackupRestored(Result<Box<crate::backup::Restored>, String>),
    #[cfg(target_os = "android")]
    PeerSynced(Result<usize, String>),
    #[cfg(target_os = "android")]
    RemoteLyrics { seq: u64, result: Result<Option<kazzu_ytm::Lyrics>, crate::catalog::ErrorKind> },
    TrackVms { seq: u64, tracks: Vec<TrackVm> },
    /// Lista "Recentes" atualizada (depois de registrar uma escuta).
    Recents(Vec<TrackVm>),
    ListeningSummary(String),
    FolderListing { seq: u64, result: Result<FolderListingVm, String> },
    /// Algo falhou: o texto já é para o usuário.
    Failed(String),
    /// Curtida gravada.
    LikedSaved { id: String, liked: bool },
    FoldersChanged,
    /// Aviso pronto para o usuário (ex.: "Adicionada à playlist").
    Notice(String),
    /// Resultado de um [`RemoteJob`].
    Remote(RemoteResult),
}

pub type Emit<'a> = &'a (dyn Fn(JobResult) + Sync);

/// Quem executa jobs. O app usa threads; os testes, execução na hora.
pub trait JobRunner {
    fn submit(&self, job: Job);
}

fn parse_id(id: &str) -> Option<i64> {
    id.parse().ok()
}

/// Miniaturas decodificadas (chave = caminho do JPEG), com limite: descarta as menos usadas, uma a uma.
struct ThumbCache {
    map: Lru<PathBuf, Option<Pixels>>,
}

impl Default for ThumbCache {
    fn default() -> Self {
        Self { map: Lru::new(THUMB_CACHE_MAX) }
    }
}

impl ThumbCache {
    fn get(&mut self, path: &Path) -> Option<Pixels> {
        let key = path.to_path_buf();
        if let Some(hit) = self.map.get(&key) {
            return hit.clone();
        }
        let px = art::load(path).map(|img| art::to_pixels(&img));
        if px.is_none() {
            log::warn!("capa local não carregou: {} (existe: {})", path.display(), path.exists());
        }
        self.map.insert(key, px.clone());
        px
    }
}

pub struct Store {
    lib: Library,
    thumbs: ThumbCache,
    albums: HashMap<i64, Album>,
    liked: HashSet<i64>,
    providers: Providers,
}

impl Store {
    pub fn open(data_dir: &Path) -> Result<Self, kazzu_library::Error> {
        Ok(Self::new(Library::open(data_dir)?))
    }

    pub fn new(lib: Library) -> Self {
        Self { lib, thumbs: ThumbCache::default(), albums: HashMap::new(), liked: HashSet::new(), providers: Providers::new() }
    }

    /// Provedores remotos cujas curtidas (guardadas no aparelho) entram em "Curtidos" e nas playlists.
    pub fn with_providers(mut self, providers: Providers) -> Self {
        self.providers = providers;
        self
    }

    /// Faixas remotas curtidas, da mais nova para a mais antiga.
    fn remote_liked(&self) -> Vec<kazzu_core::Track> {
        self.providers.default_remote().map(|c| c.liked()).unwrap_or_default()
    }

    pub fn library(&self) -> &Library {
        &self.lib
    }

    fn reload_caches(&mut self) -> Result<(), kazzu_library::Error> {
        self.albums = self.lib.albums()?.into_iter().map(|a| (a.id.0, a)).collect();
        self.liked = self.lib.liked()?.iter().map(|t| t.id.0).collect();
        Ok(())
    }

    fn artwork(&self, album: Option<AlbumId>) -> Option<&Artwork> {
        self.albums.get(&album?.0)?.artwork.as_ref()
    }

    fn thumb(&mut self, album: Option<AlbumId>) -> Option<Pixels> {
        let path = self.artwork(album)?.thumb.clone();
        self.thumbs.get(&path)
    }

    /// Capa grande (até 640 px) do álbum, para o topo do detalhe.
    fn detail_cover(&self, album: Option<AlbumId>) -> Option<Pixels> {
        let path = self.artwork(album)?.cover.clone();
        art::load(&path).map(|img| art::to_pixels(&art::fit_within(img, 640)))
    }

    fn track_vm(&mut self, t: &Track) -> TrackVm {
        let b = format::badge(&t.codec, format::is_lossless_name(&t.codec), t.bit_depth, t.sample_rate, t.bitrate);
        TrackVm {
            id: t.id.0.to_string(),
            title: t.title.clone(),
            artist: t.artist_display(),
            album: t.album.clone().unwrap_or_default(),
            cover: self.thumb(t.album_id),
            duration_label: format::duration_label_ms(t.duration_ms),
            codec: b.codec,
            detail: b.detail,
            detail_long: b.detail_long,
            hires: b.hires,
            source: String::new(),
            liked: self.liked.contains(&t.id.0),
            playing: false,
            ..Default::default()
        }
    }

    fn album_vm(&mut self, a: &Album) -> AlbumVm {
        let cover = a.artwork.as_ref().and_then(|w| self.thumbs.get(&w.thumb.clone()));
        AlbumVm { id: a.id.0.to_string(), title: a.title.clone(), artist: a.artist.clone(), cover, cover_url: None }
    }

    fn album_list(&mut self, albums: &[Album]) -> Vec<AlbumVm> {
        albums.iter().enumerate().map(|(index, album)| {
            if !cfg!(target_os = "android") || index < MOBILE_ALBUM_THUMBNAILS {
                self.album_vm(album)
            } else {
                AlbumVm {
                    id: album.id.0.to_string(), title: album.title.clone(),
                    artist: album.artist.clone(), ..Default::default()
                }
            }
        }).collect()
    }

    /// Nome para "Tocando de …" quando quem pediu não deu um.
    fn scope_label(&self, scope: &Scope) -> String {
        match scope {
            Scope::Album(id) => parse_id(id).and_then(|i| self.albums.get(&i)).map_or_else(|| "Álbum".to_owned(), |a| a.title.clone()),
            Scope::Playlist(id) => parse_id(id)
                .and_then(|i| self.lib.playlists().ok()?.into_iter().find(|p| p.id.0 == i))
                .map_or_else(|| "Playlist".to_owned(), |p| p.name),
            Scope::Liked => "Curtidos".to_owned(),
            Scope::Genre(g) => g.clone(),
            Scope::SmartRecent => "Adicionadas recentemente".to_owned(),
            Scope::SmartUnplayed => "Ainda não ouvidas".to_owned(),
            Scope::All | Scope::Ids(_) => "Biblioteca".to_owned(),
        }
    }

    /// Resolve uma `Scope` em faixas (na ordem de reprodução natural).
    fn tracks_for(&mut self, scope: &Scope) -> Result<Vec<Track>, kazzu_library::Error> {
        Ok(match scope {
            Scope::Album(id) => parse_id(id).map(|i| self.lib.album_tracks(AlbumId(i))).transpose()?.unwrap_or_default(),
            Scope::Liked => self.lib.liked()?,
            Scope::Playlist(id) => parse_id(id).map(|i| self.lib.playlist_tracks(PlaylistId(i))).transpose()?.unwrap_or_default(),
            Scope::Genre(g) => self.lib.tracks()?.into_iter().filter(|t| {
                t.genre.as_deref().is_some_and(|genre| genre.trim().eq_ignore_ascii_case(g.trim()))
            }).collect(),
            Scope::All => self.lib.tracks()?,
            Scope::SmartRecent => self.lib.recently_added(100)?,
            Scope::SmartUnplayed => self.lib.never_played(100)?,
            Scope::Ids(ids) => {
                let mut out = Vec::with_capacity(ids.len());
                for id in ids {
                    if let Some(t) = parse_id(id).map(|i| self.lib.track(TrackId(i))).transpose()?.flatten() {
                        out.push(t);
                    }
                }
                out
            }
        })
    }

    fn queue_item(&self, t: &Track) -> Option<QueueItem> {
        QueueItem::local(t.to_core(self.artwork(t.album_id)))
    }

    fn prepare_play(&mut self, scope: Scope, start_id: Option<String>, label: String) -> Result<(Vec<QueueItem>, usize, String), kazzu_library::Error> {
        self.reload_caches_if_empty()?;
        let label = if label.is_empty() { self.scope_label(&scope) } else { label };
        let tracks = self.tracks_for(&scope)?;
        let playable: Vec<_> = tracks.iter().filter_map(|track| self.queue_item(track).map(|item| (track, item))).collect();
        let start = start_id.and_then(|id| playable.iter().position(|(track, _)| track.id.0.to_string() == id)).unwrap_or(0);
        Ok((playable.into_iter().map(|(_, item)| item).collect(), start, label))
    }

    /// Executa um job. `emit` entrega resultados (pode ser chamado mais de uma vez, ex.: progresso do scan).
    pub fn run(&mut self, job: Job, emit: Emit<'_>) {
        if let Err(e) = self.run_inner(job, emit) {
            log::warn!("job falhou: {e}");
            emit(JobResult::Failed(format!("Não foi possível ler a biblioteca. {e}")));
        }
    }

    fn run_inner(&mut self, job: Job, emit: Emit<'_>) -> Result<(), kazzu_library::Error> {
        match job {
            Job::Refresh => {
                let snap = self.snapshot()?;
                emit(JobResult::Refreshed(Box::new(snap)));
            }
            Job::Scan => {
                let result = scan(&mut self.lib, emit);
                emit(JobResult::ScanDone(result));
            }
            Job::AddFolder(path) => {
                if !path.is_dir() {
                    emit(JobResult::Failed(format!("A pasta {} não existe mais.", path.display())));
                } else {
                    self.lib.add_folder(&path)?;
                    emit(JobResult::FoldersChanged);
                }
            }
            Job::RemoveFolder(path) => {
                self.lib.remove_folder(&path)?;
                emit(JobResult::FoldersChanged);
            }
            Job::BrowseFolders { seq, path, root } => {
                emit(JobResult::FolderListing { seq, result: list_folders(&path, &root) });
            }
            Job::ImportAccount { lists, skipped } => {
                let result = lists.iter().try_for_each(|list| self.lib.import_remote_playlist(&list.key, &list.name, &list.tracks));
                emit(JobResult::AccountImported(result.map(|()| format!("{} coleções importadas, {skipped} listas sem acesso.", lists.len()))
                    .map_err(|error| format!("A gravação parou. Coleções já gravadas foram preservadas. {error}"))));
                emit(JobResult::Refreshed(Box::new(self.snapshot()?)));
            }
            Job::Search { seq, query } => {
                self.reload_caches_if_empty()?;
                let found = self.lib.search(&query, SEARCH_LIMIT)?;
                let songs: Vec<TrackVm> = found.tracks.iter().map(|t| self.track_vm(t)).collect();
                let ids = songs.iter().map(|s| s.id.clone()).collect();
                let albums = found.albums.iter().map(|a| self.album_vm(a)).collect();
                emit(JobResult::Search { seq, songs, albums, ids });
            }
            Job::Play { scope, start_id, shuffle, label } => {
                let (items, start, label) = self.prepare_play(scope, start_id, label)?;
                if items.is_empty() {
                    emit(JobResult::Failed("Não há músicas para tocar aqui.".into()));
                } else {
                    emit(JobResult::Queue { items, start, shuffle, label });
                }
            }
            Job::PlayScoped { seq, scope, start_id, shuffle, label } => {
                let (items, start, label) = self.prepare_play(scope, start_id, label)?;
                emit(JobResult::PreparedQueue { seq, items, start, shuffle, label });
            }
            Job::PrepareQueue { seq, entries, start, shuffle, label } => {
                self.reload_caches_if_empty()?;
                let mut items = Vec::with_capacity(entries.len());
                let mut playable_start = None;
                for (index, entry) in entries.into_iter().enumerate() {
                    let item = match entry {
                        crate::controller::Entry::Local(id) => match parse_id(&id) {
                            Some(id) => self.lib.track(TrackId(id))?.as_ref().and_then(|t| self.queue_item(t)),
                            None => None,
                        },
                        crate::controller::Entry::Remote(track) => {
                            Some(QueueItem::new(*track, kazzu_core::StreamSource::Deferred))
                        }
                    };
                    if let Some(item) = item {
                        if index >= start && playable_start.is_none() {
                            playable_start = Some(items.len());
                        }
                        items.push(item);
                    }
                }
                let start = playable_start.unwrap_or_else(|| items.len().saturating_sub(1));
                emit(JobResult::PreparedQueue { seq, items, start, shuffle, label });
            }
            Job::QueueItems { id, mode } => {
                self.reload_caches_if_empty()?;
                let tracks = self.tracks_for(&Scope::Ids(vec![id]))?;
                let items: Vec<QueueItem> = tracks.iter().filter_map(|t| self.queue_item(t)).collect();
                if !items.is_empty() {
                    emit(JobResult::QueueItems { items, mode });
                }
            }
            Job::OpenDetail { seq, what } => {
                match self.reload_caches_if_empty().and_then(|()| self.detail(&what)) {
                    Ok((detail, ids, remote_tracks)) => emit(JobResult::Detail { seq, detail: Box::new(detail), ids, remote_tracks }),
                    Err(error) => emit(JobResult::DetailFailed { seq, message: format!("Não foi possível abrir esta coleção. {error}") }),
                }
            }
            #[cfg(target_os = "android")]
            Job::Lyrics { seq, path } => emit(JobResult::Lyrics { seq, lines: crate::lyrics::read_lrc(&path) }),
            #[cfg(target_os = "android")]
            Job::ListeningStats { seq, from, until, export } => {
                let stats = Box::new(self.lib.listening_stats(from, until)?);
                emit(JobResult::ListeningStats { seq, stats, export });
            }
            #[cfg(target_os = "android")]
            Job::EditLocalTags { root, track_id, title, artist, album, genre } => {
                let result = (|| {
                    let id = parse_id(&track_id).ok_or("Faixa local inválida.")?;
                    let track = self.lib.track(TrackId(id)).map_err(|e| e.to_string())?.ok_or("Faixa não encontrada.")?;
                    let path = track.path.canonicalize().map_err(|e| e.to_string())?;
                    let root = root.canonicalize().map_err(|e| e.to_string())?;
                    if !path.starts_with(&root) {
                        return Err("A edição é permitida em cópias importadas para o Kazzu.".to_owned());
                    }
                    kazzu_library::update_tags(&path, &title, &artist, &album, &genre).map_err(|e| e.to_string())
                })();
                emit(JobResult::LocalTagsEdited(result));
            }
            #[cfg(target_os = "android")]
            Job::ImportM3u { title, contents } => {
                let (ids, skipped) = crate::m3u::resolve(&contents, self.lib.tracks()?);
                if ids.is_empty() {
                    emit(JobResult::Failed("Nenhuma faixa do M3U corresponde a uma música da biblioteca. Importe a pasta de áudio primeiro.".into()));
                } else {
                    let title = title.trim();
                    let title = if title.is_empty() { "Playlist importada" } else { title };
                    self.lib.import_local_playlist(title, &ids)?;
                    emit(JobResult::M3uImported { count: ids.len(), skipped });
                }
            }
            #[cfg(target_os = "android")]
            Job::ExportBackup { root, config, lan, room } => {
                emit(JobResult::BackupReady { result: crate::backup::export(&self.lib, &root, *config), lan, room });
            }
            #[cfg(target_os = "android")]
            Job::RestoreBackup { root, json } => {
                emit(JobResult::BackupRestored(crate::backup::restore(&mut self.lib, &root, &json).map(Box::new)));
            }
            #[cfg(target_os = "android")]
            Job::SyncPeerPlaylists { root, json } => {
                emit(JobResult::PeerSynced(crate::backup::sync_playlists(&mut self.lib, &root, &json)));
            }
            #[cfg(target_os = "android")]
            Job::SetPlaylistCover { id, bytes } => {
                match crate::playlist_cover::encode(&bytes) {
                    Ok(image) => {
                        let playlist = PlaylistId(parse_id(&id).ok_or(kazzu_library::Error::NotFound("playlist"))?);
                        self.lib.set_playlist_cover(playlist, &image)?;
                        emit(JobResult::PlaylistEdited { id, deleted: false });
                    }
                    Err(error) => emit(JobResult::Failed(error)),
                }
            }
            #[cfg(target_os = "android")]
            Job::OpenCachedAudio { seq, cache } => {
                let complete = cache.complete_keys();
                let mut seen = HashSet::new();
                let entries = self.lib.known_remote_tracks()?.into_iter().chain(self.remote_liked())
                    .filter(|track| kazzu_audio::StreamCache::key_for(track).is_some_and(|key| complete.contains(&key) && seen.insert(key)))
                    .map(PlaylistEntry::Remote).collect::<Vec<_>>();
                let (mut detail, ids, remote_tracks) = self.mixed_detail(
                    "Áudio em cache".into(), "Nenhuma faixa com cópia completa no cache.", entries,
                )?;
                detail.cached_audio = true;
                detail.subtitle = "Cópias temporárias para reprodução offline".into();
                emit(JobResult::Detail { seq, detail: Box::new(detail), ids, remote_tracks });
            }
            Job::NowArt { seq, cover } => {
                let art = match cover.as_deref().and_then(art::load) {
                    Some(img) => {
                        let (player_dim, mini_dim) = art::glass_dims(&img);
                        NowArtVm {
                            accent: accent::derive(&img),
                            backdrop: Some(art::to_pixels(&art::backdrop(&img))),
                            backdrop_wide: Some(art::to_pixels(&art::backdrop_wide(&img))),
                            player_dim,
                            mini_dim,
                            thumb: Some(art::to_pixels(&art::fit_within(img.clone(), 192))),
                            cover: Some(art::to_pixels(&art::fit_within(img, 640))),
                        }
                    }
                    None => NowArtVm::default(),
                };
                emit(JobResult::NowArt { seq, art: Box::new(art) });
            }
            Job::TrackVms { seq, ids } => {
                self.reload_caches_if_empty()?;
                let tracks = self.tracks_for(&Scope::Ids(ids))?;
                let vms = tracks.iter().map(|t| self.track_vm(t)).collect();
                emit(JobResult::TrackVms { seq, tracks: vms });
            }
            Job::SetLiked { id, liked } => {
                if let Some(tid) = parse_id(&id) {
                    if liked { self.lib.like(TrackId(tid))? } else { self.lib.unlike(TrackId(tid))? }
                    if liked { self.liked.insert(tid); } else { self.liked.remove(&tid); }
                    emit(JobResult::LikedSaved { id, liked });
                }
            }
            Job::RecordRemotePlay { track, listened_ms } => {
                self.lib.record_remote_play(&track, listened_ms)?;
                emit(JobResult::ListeningSummary(self.listening_summary()?));
            }
            Job::RecordPlay { id, listened_ms } => {
                if let Some(tid) = parse_id(&id) {
                    self.lib.record_play(TrackId(tid), listened_ms)?;
                    self.reload_caches_if_empty()?;
                    let recents = self.recent_tracks()?.iter().map(|t| self.track_vm(t)).collect();
                    emit(JobResult::Recents(recents));
                    emit(JobResult::ListeningSummary(self.listening_summary()?));
                }
            }
            Job::EditPlaylist { id, edit } => {
                let pid = PlaylistId(parse_id(&id).ok_or(kazzu_library::Error::NotFound("playlist"))?);
                let deleted = matches!(edit, PlaylistEdit::Delete);
                match edit {
                    PlaylistEdit::Rename(name) => self.lib.rename_playlist(pid, name.trim())?,
                    PlaylistEdit::Delete => self.lib.delete_playlist(pid)?,
                    PlaylistEdit::Remove { index, ref track } | PlaylistEdit::MoveUp { index, ref track } => {
                        let entries = self.lib.playlist_entries(pid)?;
                        let current = entries.get(index).map(|entry| match entry {
                            PlaylistEntry::Local(track) => track.id.0.to_string(),
                            PlaylistEntry::Remote(track) => ytm_vm::track_ui_id(track),
                        });
                        if current.as_ref() != Some(track) {
                            emit(JobResult::Failed("A lista mudou. Abra-a novamente antes de editar.".into()));
                            return Ok(());
                        }
                        if matches!(edit, PlaylistEdit::Remove { .. }) {
                            self.lib.remove_from_playlist(pid, index)?;
                        } else if index > 0 {
                            self.lib.reorder_playlist(pid, index, index - 1)?;
                        }
                    }
                }
                emit(JobResult::PlaylistEdited { id, deleted });
                emit(JobResult::Refreshed(Box::new(self.snapshot()?)));
            }
            Job::CreatePlaylist { name, add } => {
                let name = name.trim();
                if name.is_empty() {
                    emit(JobResult::Failed("Dê um nome à playlist.".into()));
                } else {
                    let id = self.lib.create_playlist(name)?;
                    if let Some((track_id, remote)) = add {
                        self.add_to_playlist(id, &track_id, remote.as_deref())?;
                    }
                    let snap = self.snapshot()?;
                    emit(JobResult::Refreshed(Box::new(snap)));
                    emit(JobResult::Notice("Playlist criada".into()));
                }
            }
            Job::AddToPlaylist { playlist_id, track_id, remote } => match parse_id(&playlist_id) {
                Some(pid) => {
                    self.add_to_playlist(PlaylistId(pid), &track_id, remote.as_deref())?;
                    let snap = self.snapshot()?;
                    emit(JobResult::Refreshed(Box::new(snap)));
                    emit(JobResult::Notice("Adicionada à playlist".into()));
                }
                None => emit(JobResult::Failed("Não encontrei essa playlist.".into())),
            },
            Job::ClearHistory => {
                self.lib.clear_history()?;
                let recents = self.recent_tracks()?.iter().map(|t| self.track_vm(t)).collect();
                emit(JobResult::Recents(recents));
                    emit(JobResult::ListeningSummary(self.listening_summary()?));
                emit(JobResult::Notice("Histórico de escuta apagado".into()));
            }
            Job::SaveConfig { path, config } => {
                if let Err(e) = config.save(&path) {
                    log::warn!("{e}");
                    emit(JobResult::Failed("Não foi possível salvar as configurações.".into()));
                }
            }
            Job::WriteReport { path, text } => match std::fs::write(&path, text) {
                Ok(()) => emit(JobResult::Notice(format!("Relato salvo em {}. Nada foi enviado", path.display()))),
                Err(e) => emit(JobResult::Failed(format!("Não deu para salvar o relato. {e}"))),
            },
            Job::ResolveLocal { ctx_seq, id, for_start } => {
                let found = self.reload_caches_if_empty().and_then(|()| self.tracks_for(&Scope::Ids(vec![id.clone()])));
                let item = found.ok().and_then(|tracks| tracks.first().and_then(|t| self.queue_item(t)));
                match item {
                    Some(item) => emit(JobResult::Remote(RemoteResult::Resolved { ctx_seq, item: Box::new(item), quality_label: String::new(), for_start })),
                    None => {
                        // o arquivo sumiu da biblioteca: a fila precisa saber, senão ficaria esperando para sempre
                        let track = kazzu_core::Track::new(id, "Arquivo do aparelho", kazzu_core::TrackSource::Local { path: PathBuf::new() });
                        emit(JobResult::Remote(RemoteResult::ResolveFailed { ctx_seq, track: Box::new(track), kind: crate::catalog::ErrorKind::NotFound, for_start }));
                    }
                }
            }
            // a rede não passa pela thread da biblioteca: `ThreadedRunner` desvia estes jobs antes de chegar aqui
            Job::Remote(_) => emit(JobResult::Failed("Este trabalho de rede foi enviado ao lugar errado.".into())),
        }
        Ok(())
    }

    fn add_to_playlist(&mut self, id: PlaylistId, track_id: &str, remote: Option<&kazzu_core::Track>) -> Result<(), kazzu_library::Error> {
        match remote {
            Some(t) => self.lib.add_remote_to_playlist(id, std::slice::from_ref(t)),
            None => match parse_id(track_id) {
                Some(t) => self.lib.add_to_playlist(id, &[TrackId(t)]),
                None => Ok(()),
            },
        }
    }

    fn listening_summary(&self) -> Result<String, kazzu_library::Error> {
        let (minutes, plays) = self.lib.listening_totals()?;
        Ok(format!("{} min · {} reproduções", format::thousands(minutes), format::thousands(plays)))
    }

    fn reload_caches_if_empty(&mut self) -> Result<(), kazzu_library::Error> {
        if self.albums.is_empty() {
            self.reload_caches()?;
        }
        Ok(())
    }

    /// Tudo que Início, Biblioteca e Configurações mostram, de uma vez.
    fn snapshot(&mut self) -> Result<Snapshot, kazzu_library::Error> {
        self.reload_caches()?;
        let folders = self.lib.folders()?;
        let track_count = self.lib.track_count()?;
        let all_albums = self.lib.albums()?;

        let recent = self.recent_tracks()?;
        let recents: Vec<TrackVm> = recent.iter().map(|t| self.track_vm(t)).collect();

        // Álbuns: primeiro os dos recentes, depois os demais.
        let mut order: Vec<&Album> = Vec::new();
        let mut seen = HashSet::new();
        for t in &recent {
            if let Some(a) = t.album_id.and_then(|id| self.albums.get(&id.0))
                && seen.insert(a.id.0)
            {
                order.push(a);
            }
        }
        for a in &all_albums {
            if seen.insert(a.id.0) {
                order.push(a);
            }
        }
        let order: Vec<Album> = order.into_iter().take(HOME_ALBUMS).cloned().collect();
        let albums: Vec<AlbumVm> = order.iter().map(|a| self.album_vm(a)).collect();

        let liked_tracks = self.lib.liked()?;
        let remote_liked = self.remote_liked();

        let playlists = self.lib.playlists()?;
        let mut playlist_vms = Vec::with_capacity(playlists.len());
        for p in &playlists {
            let tracks = self.lib.playlist_tracks(p.id)?;
            playlist_vms.push(PlaylistVm {
                id: p.id.0.to_string(),
                name: p.name.clone(),
                count_label: format::songs_label(u64::from(p.track_count)),
                covers: if let Some(cover) = self.playlist_cover(p.id)? { vec![cover] } else { self.distinct_covers(&tracks, 4) },
            });
        }

        let covers_of_albums: Vec<Pixels> = all_albums.iter().filter_map(|a| self.album_vm(a).cover).take(3).collect();
        let liked_covers = self.distinct_covers(&liked_tracks, 1);
        let collections = vec![
            CollectionVm { id: "history".into(), title: "Ouvidas recentemente".into(), count_label: "Histórico local".into(), covers: vec![], soon: false },
            CollectionVm { id: "liked".into(), title: "Curtidos".into(), count_label: format::songs_label((liked_tracks.len() + remote_liked.len()) as u64), covers: liked_covers, soon: false },
            CollectionVm { id: "albums".into(), title: "Álbuns".into(), count_label: format::albums_label(all_albums.len() as u64), covers: covers_of_albums, soon: false },
            CollectionVm {
                id: "playlists".into(),
                title: "Playlists".into(),
                count_label: format::playlists_label(playlists.len() as u64),
                covers: playlist_vms.iter().flat_map(|p| p.covers.iter().cloned()).take(4).collect(),
                soon: false,
            },
        ];

        // Fundo ambiente enquanto nada toca: o primeiro álbum.
        let ambient = all_albums
            .first()
            .and_then(|a| a.artwork.as_ref())
            .and_then(|w| art::load(&w.cover))
            .map(|img| art::to_pixels(&art::backdrop_wide(&img)));

        Ok(Snapshot {
            home: HomeVm { recents, albums, ..Default::default() },
            library: LibraryVm { #[cfg(target_os = "android")] genres: self.lib.genres()?, listening_summary: self.listening_summary()?, albums: self.album_list(&all_albums), artists: self.lib.artists()?.into_iter().map(|a| ArtistVm { id: a.id.0.to_string(), name: a.name, subtitle: format::songs_label(a.track_count as u64), ..Default::default() }).collect(), collections, playlists: playlist_vms },
            settings: SettingsVm {
                folders: folders.iter().map(|p| p.display().to_string()).collect(),
                summary: format::library_summary(folders.len(), track_count),
                has_library: track_count > 0,
                track_count,
            },
            ambient,
            liked: self.liked.iter().map(|i| i.to_string()).chain(remote_liked.iter().map(|t| ytm_vm::remote_id(t.id.as_str()))).collect(),
            remote_liked,
        })
    }

    /// Recentes: o que foi ouvido por último; o que falta para completar a lista vem do que entrou há pouco.
    fn recent_tracks(&self) -> Result<Vec<Track>, kazzu_library::Error> {
        let mut out = self.lib.recent(RECENTS)?;
        if out.len() < RECENTS {
            let seen: HashSet<i64> = out.iter().map(|t| t.id.0).collect();
            out.extend(self.lib.recently_added(RECENTS)?.into_iter().filter(|t| !seen.contains(&t.id.0)));
            out.truncate(RECENTS);
        }
        Ok(out)
    }

    fn distinct_covers(&mut self, tracks: &[Track], max: usize) -> Vec<Pixels> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for t in tracks {
            let Some(aid) = t.album_id else { continue };
            if seen.insert(aid.0)
                && let Some(px) = self.thumb(Some(aid))
            {
                out.push(px);
                if out.len() == max {
                    break;
                }
            }
        }
        out
    }

    /// Detalhe + ids das faixas (para tocar na ordem exibida).
    fn playlist_cover(&self, id: PlaylistId) -> Result<Option<Pixels>, kazzu_library::Error> {
        Ok(self.lib.playlist_cover(id)?.and_then(|png| {
            image::load_from_memory_with_format(&png, image::ImageFormat::Png)
                .ok().map(|image| art::to_pixels(&image.into_rgba8()))
        }))
    }

    fn detail(&mut self, r: &DetailRef) -> Result<(DetailVm, Vec<String>, Vec<kazzu_core::Track>), kazzu_library::Error> {
        if let DetailRef::ArtistOfTrack(track_id) = r {
            let name = parse_id(track_id).and_then(|id| self.lib.track(TrackId(id)).ok().flatten()).and_then(|t| t.artists.first().cloned());
            let found = self.lib.artists()?.into_iter().find(|a| Some(a.name.as_str()) == name.as_deref());
            return match found {
                Some(a) => self.detail(&DetailRef::Artist(a.id.0.to_string())),
                None => Ok((DetailVm { state: LoadStateVm::Empty, title: "Artista".into(), empty_text: "Este arquivo não informa o artista.".into(), ..Default::default() }, vec![], vec![])),
            };
        }
        if let DetailRef::Artist(id) = r {
            let artist_id = ArtistId(parse_id(id).unwrap_or(-1));
            let title = self.lib.artists()?.into_iter().find(|a| a.id == artist_id).map(|a| a.name).unwrap_or_else(|| "Artista".into());
            let tracks = self.lib.artist_tracks(artist_id)?;
            let albums = self.lib.artist_albums(artist_id)?;
            let mut out = self.mixed_detail(title, "Adicione músicas deste artista à biblioteca.", tracks.into_iter().map(PlaylistEntry::Local).collect())?;
            out.0.kind = DetailKindVm::Artist;
            out.0.albums = self.album_list(&albums);
            return Ok(out);
        }
        if let DetailRef::AlbumOfTrack(track_id) = r {
            let album = parse_id(track_id).and_then(|i| self.lib.track(TrackId(i)).ok().flatten()).and_then(|t| t.album_id);
            return match album {
                Some(a) => self.detail(&DetailRef::Album(a.0.to_string())),
                None => Ok((DetailVm { state: LoadStateVm::Empty, empty_text: "Esta música não tem álbum.".into(), title: "Álbum".into(), ..Default::default() }, vec![], vec![])),
            };
        }
        let (title, subtitle, empty, scope) = match r {
            DetailRef::History => {
                let entries = self.lib.recent_mixed(200)?;
                return self.mixed_detail("Ouvidas recentemente".into(), "O histórico aparece após ouvir músicas.", entries);
            }
            DetailRef::SmartRecent => ("Adicionadas recentemente".into(), String::new(), "Nenhuma música indexada.".into(), Scope::SmartRecent),
            DetailRef::SmartUnplayed => ("Ainda não ouvidas".into(), String::new(), "Nenhuma música sem reprodução registrada.".into(), Scope::SmartUnplayed),
            DetailRef::Songs => ("Músicas".into(), String::new(), "Adicione uma pasta para ouvir suas músicas aqui.".into(), Scope::All),
            DetailRef::AlbumOfTrack(_) | DetailRef::ArtistOfTrack(_) | DetailRef::Artist(_) => unreachable_detail(),
            DetailRef::Album(id) => {
                let album = parse_id(id).and_then(|i| self.albums.get(&i).cloned());
                match album {
                    Some(a) => {
                        let sub = format!("{}{}", a.artist, a.year.map(|y| format!(", {y}")).unwrap_or_default());
                        (a.title, sub, "Este álbum não tem faixas indexadas.".to_owned(), Scope::Album(id.clone()))
                    }
                    None => ("Álbum".into(), String::new(), "Não encontramos este álbum. Ele pode ter sido removido da pasta.".into(), Scope::Album(id.clone())),
                }
            }
            DetailRef::FavoriteMix => {
                use std::collections::{BTreeMap, VecDeque};
                let local = self.lib.liked()?.into_iter().map(PlaylistEntry::Local);
                let remote = self.remote_liked().into_iter().map(PlaylistEntry::Remote);
                let mut by_artist = BTreeMap::<String, VecDeque<PlaylistEntry>>::new();
                for entry in local.chain(remote) {
                    let artist = match &entry {
                        PlaylistEntry::Local(track) => track.artists.first().cloned(),
                        PlaylistEntry::Remote(track) => track.artists.first().map(|artist| artist.name.clone()),
                    }.unwrap_or_default().to_lowercase();
                    by_artist.entry(artist).or_default().push_back(entry);
                }
                let mut entries = Vec::new();
                while !by_artist.is_empty() {
                    by_artist.retain(|_, tracks| {
                        if let Some(track) = tracks.pop_front() { entries.push(track); }
                        !tracks.is_empty()
                    });
                }
                return self.mixed_detail("Meu mix de curtidas".into(), "Curta faixas para montar seu mix.", entries);
            }
            DetailRef::Liked => {
                // curtidas remotas (mais novas primeiro) e depois as locais
                let mut entries: Vec<PlaylistEntry> = self.remote_liked().into_iter().map(PlaylistEntry::Remote).collect();
                entries.extend(self.lib.liked()?.into_iter().map(PlaylistEntry::Local));
                let mut out = self.mixed_detail("Curtidos".into(), "Toque no coração de uma música para ela aparecer aqui. Fica só neste aparelho.", entries)?;
                out.0.subtitle = if out.0.tracks.is_empty() { String::new() } else { format!("{}. Fica só neste aparelho", out.0.subtitle) };
                return Ok(out);
            }
            DetailRef::Downloads => ("Baixados".into(), String::new(), "O modo offline com downloads chega em uma próxima versão.".into(), Scope::Ids(vec![])),
            DetailRef::Albums => {
                let mut albums: Vec<Album> = self.albums.values().cloned().collect();
                albums.sort_by_key(|a| a.title.to_lowercase());
                let vms = self.album_list(&albums);
                let sub = format::albums_label(vms.len() as u64);
                return Ok((
                    DetailVm { kind: DetailKindVm::Albums, state: LoadStateVm::Ready, title: "Álbuns".into(), subtitle: sub, empty_text: "Adicione uma pasta de músicas para ver seus álbuns.".into(), albums: vms, ..Default::default() },
                    vec![],
                    vec![],
                ));
            }
            DetailRef::Playlist(id) => {
                let name = parse_id(id)
                    .and_then(|i| self.lib.playlists().ok()?.into_iter().find(|p| p.id.0 == i))
                    .map(|p| p.name)
                    .unwrap_or_else(|| "Playlist".into());
                let entries = self.lib.playlist_entries(PlaylistId(parse_id(id).ok_or(kazzu_library::Error::NotFound("playlist"))?))?;
                let (mut detail, tracks, remote) = self.mixed_detail(name, "Esta playlist ainda está vazia.", entries)?;
                if let Some(cover) = self.playlist_cover(PlaylistId(parse_id(id).unwrap_or(-1)))? {
                    detail.cover = Some(cover);
                }
                detail.editable_playlist = id.clone();
                return Ok((detail, tracks, remote));
            }
            DetailRef::Genre(g) => (g.clone(), String::new(), "Nenhuma música com este gênero.".into(), Scope::Genre(g.clone())),
        };
        let tracks = self.tracks_for(&scope)?;
        let total_ms: u64 = tracks.iter().map(|t| t.duration_ms).sum();
        let vms: Vec<TrackVm> = tracks.iter().map(|t| self.track_vm(t)).collect();
        let ids = vms.iter().filter(|t| !t.id.starts_with("sp:")).map(|t| t.id.clone()).collect();
        let subtitle = if subtitle.is_empty() && !vms.is_empty() {
            format!("{}, {} min", format::songs_label(vms.len() as u64), format::thousands(total_ms.div_ceil(60_000)))
        } else {
            subtitle
        };
        let state = if vms.is_empty() { LoadStateVm::Empty } else { LoadStateVm::Ready };
        let cover = tracks.iter().find_map(|t| t.album_id.filter(|a| self.artwork(Some(*a)).is_some())).and_then(|a| self.detail_cover(Some(a)));
        Ok((DetailVm { kind: DetailKindVm::Tracks, state, title, subtitle, empty_text: empty, tracks: vms, cover, ..Default::default() }, ids, vec![]))
    }

    /// Detalhe de uma lista que mistura faixas locais e remotas (playlists, curtidas), na ordem dada.
    fn mixed_detail(&mut self, title: String, empty: &str, entries: Vec<PlaylistEntry>) -> Result<(DetailVm, Vec<String>, Vec<kazzu_core::Track>), kazzu_library::Error> {
        let mut vms = Vec::with_capacity(entries.len());
        let mut remote = Vec::new();
        let mut total_ms = 0u64;
        let mut first_local_album: Option<AlbumId> = None;
        for e in entries {
            match e {
                PlaylistEntry::Local(t) => {
                    total_ms += t.duration_ms;
                    if first_local_album.is_none() && t.album_id.is_some_and(|a| self.artwork(Some(a)).is_some()) {
                        first_local_album = t.album_id;
                    }
                    vms.push(self.track_vm(&t));
                }
                PlaylistEntry::Remote(t) => {
                    total_ms += t.duration.map_or(0, |d| d.as_millis() as u64);
                    vms.push(ytm_vm::track_vm(&t, false, ""));
                    remote.push(t);
                }
            }
        }
        let ids = vms.iter().filter(|t| !t.id.starts_with("sp:")).map(|t| t.id.clone()).collect();
        let subtitle = if vms.is_empty() { String::new() } else { format!("{}, {} min", format::songs_label(vms.len() as u64), format::thousands(total_ms.div_ceil(60_000))) };
        let state = if vms.is_empty() { LoadStateVm::Empty } else { LoadStateVm::Ready };
        Ok((DetailVm { kind: DetailKindVm::Tracks, state, title, subtitle, empty_text: empty.to_owned(), tracks: vms, cover: first_local_album.and_then(|a| self.detail_cover(Some(a))), ..Default::default() }, ids, remote))
    }
}

/// `DetailRef::AlbumOfTrack` já foi tratado antes do `match`; este ramo só existe para o compilador.
fn unreachable_detail() -> (String, String, String, Scope) {
    (String::new(), String::new(), String::new(), Scope::Ids(vec![]))
}

/// Scan com progresso. Roda na thread que chamar (o app usa uma thread dedicada).
pub fn scan(lib: &mut Library, emit: Emit<'_>) -> Result<ScanReport, String> {
    let last_pct = AtomicU32::new(u32::MAX);
    let report = lib.scan(&|p| {
        let vm = match p {
            ScanProgress::Discovering { files_found } => ScanVm { progress: -1.0, label: format!("Procurando músicas, {} arquivos", format::thousands(files_found as u64)) },
            ScanProgress::Reading { done, total } => ScanVm {
                progress: if total == 0 { 0.7 } else { 0.7 * done as f32 / total as f32 },
                label: format!("Lendo tags, {} de {}", format::thousands(done as u64), format::thousands(total as u64)),
            },
            ScanProgress::Artwork { done, total } => ScanVm {
                progress: if total == 0 { 1.0 } else { 0.7 + 0.3 * done as f32 / total as f32 },
                label: format!("Preparando capas, {} de {}", format::thousands(done as u64), format::thousands(total as u64)),
            },
            ScanProgress::Finished(_) => return,
        };
        // no máximo uma atualização por ponto percentual (e uma por etapa indeterminada)
        let pct = if vm.progress < 0.0 { 1000 } else { (vm.progress * 100.0) as u32 };
        if vm.progress >= 0.0 && last_pct.swap(pct, Ordering::Relaxed) == pct {
            return;
        }
        emit(JobResult::ScanProgress(vm));
    });
    report.map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------------------------
// Execução em threads
// ---------------------------------------------------------------------------------------------

/// Thread do `Store` + thread de scan dedicada + despachante dos jobs de rede.
pub struct ThreadedRunner {
    tx: Arc<Sender<Job>>,
    remote_tx: Sender<RemoteJob>,
    remote_running: Arc<AtomicBool>,
}

impl ThreadedRunner {
    /// `emit` é chamado das threads de trabalho; deve só empurrar para a UI (ex.: `invoke_from_event_loop`).
    /// `remote`: rede dos provedores (capas, Início, busca, streams); `None` = só biblioteca local.
    pub fn spawn(data_dir: PathBuf, emit: Arc<dyn Fn(JobResult) + Send + Sync>, remote: Option<Arc<RemoteWorker>>) -> std::io::Result<Self> {
        let cache_dir = data_dir.join("cache");
        Self::spawn_with_cache(data_dir, cache_dir, emit, remote)
    }

    /// Keep disposable library artwork separate from persistent database files.
    pub fn spawn_with_cache(data_dir: PathBuf, cache_dir: PathBuf, emit: Arc<dyn Fn(JobResult) + Send + Sync>, remote: Option<Arc<RemoteWorker>>) -> std::io::Result<Self> {
        let (tx, rx) = crossbeam_channel::unbounded::<Job>();
        let (remote_tx, remote_rx) = crossbeam_channel::unbounded::<RemoteJob>();
        let scanning = Arc::new(AtomicBool::new(false));
        // Scan pedido durante outro scan (pasta adicionada/removida no meio): roda de novo quando o atual termina.
        let rescan_pending = Arc::new(AtomicBool::new(false));
        let tx = Arc::new(tx);
        let own_tx = Arc::downgrade(&tx);
        let providers = remote.as_ref().map(|r| r.providers().clone()).unwrap_or_default();
        let store_emit = emit.clone();
        std::thread::Builder::new().name("kazzu-library".into()).spawn(move || {
            let emit = store_emit;
            let opened = std::fs::create_dir_all(&data_dir)
                .map_err(|e| e.to_string())
                .and_then(|()| Library::open_with(data_dir.join("library.sqlite"), cache_dir.clone()).map_err(|e| e.to_string()));
            let mut store = match opened {
                Ok(lib) => Store::new(lib).with_providers(providers),
                Err(e) => {
                    emit(JobResult::Failed(format!("Não foi possível abrir a biblioteca. {e}")));
                    return;
                }
            };
            let mut queue = JobQueue::new(rx, SEARCH_DEBOUNCE);
            while let Some(job) = queue.next_job() {
                match job {
                    Job::Scan => {
                        if scanning.swap(true, Ordering::SeqCst) {
                            rescan_pending.store(true, Ordering::SeqCst);
                            continue;
                        }
                        let (scan_emit, dir, cache, flag, pending, again) = (emit.clone(), data_dir.clone(), cache_dir.clone(), scanning.clone(), rescan_pending.clone(), own_tx.clone());
                        let spawned = std::thread::Builder::new().name("kazzu-scan".into()).spawn(move || {
                            let result = match Library::open_with(dir.join("library.sqlite"), cache) {
                                Ok(mut lib) => scan(&mut lib, &*scan_emit),
                                Err(e) => Err(e.to_string()),
                            };
                            flag.store(false, Ordering::SeqCst);
                            scan_emit(JobResult::ScanDone(result));
                            if pending.swap(false, Ordering::SeqCst)
                                && let Some(sender) = again.upgrade()
                            {
                                let _ = sender.send(Job::Scan);
                            }
                        });
                        if let Err(e) = spawned {
                            scanning.store(false, Ordering::SeqCst);
                            emit(JobResult::Failed(format!("Não foi possível iniciar a leitura da biblioteca. {e}")));
                        }
                    }
                    other => store.run(other, &*emit),
                }
            }
        })?;

        // Rede: uma thread despacha (com debounce de busca e sugestões) para um POOL pequeno de trabalhadores, então um
        // Início lento nunca atrasa tocar uma faixa, o número de conexões é limitado e nada toca a thread da
        // biblioteca nem a da UI. Job de uma geração velha (busca/Início/detalhe já substituídos) é descartado antes
        // de começar.
        let (pool_tx, pool_rx) = crossbeam_channel::unbounded::<RemoteJob>();
        let latest = Arc::new(LatestSeq::default());
        let remote_running = Arc::new(AtomicBool::new(true));
        for n in 0..REMOTE_WORKERS {
            let (rx, worker, emit, latest) = (pool_rx.clone(), remote.clone(), emit.clone(), latest.clone());
            let running = remote_running.clone();
            std::thread::Builder::new().name(format!("kazzu-remote-{n}")).spawn(move || {
                while let Ok(job) = rx.recv() {
                    if !running.load(Ordering::Acquire) {
                        break;
                    }
                    if latest.is_stale(&job) {
                        log::debug!("trabalho de rede descartado (já substituído)");
                        continue;
                    }
                    match &worker {
                        Some(w) => w.run(job, &*emit),
                        None => emit(JobResult::Failed("Nenhum provedor de música está ativo.".into())),
                    }
                }
            })?;
        }
        std::thread::Builder::new().name("kazzu-remote".into()).spawn(move || {
            let mut queue = RemoteQueue::new(remote_rx, REMOTE_DEBOUNCE);
            while let Some(job) = queue.next_job() {
                latest.note(&job);
                if pool_tx.send(job).is_err() {
                    break;
                }
            }
        })?;
        Ok(Self { tx, remote_tx, remote_running })
    }
}

impl Drop for ThreadedRunner {
    fn drop(&mut self) {
        self.remote_running.store(false, Ordering::Release);
    }
}

impl JobRunner for ThreadedRunner {
    fn submit(&self, job: Job) {
        match job {
            Job::Remote(r) => {
                let _ = self.remote_tx.send(r);
            }
            other => {
                let _ = self.tx.send(other);
            }
        }
    }
}

/// Trabalhadores de rede simultâneos (conexões e memória limitadas, inclusive no Android).
pub const REMOTE_WORKERS: usize = 5;

/// Última geração (`seq`) vista de cada tipo de pedido que a tela substitui: um job de geração menor que a última
/// já enviada ao pool é lixo e não vale a rede.
#[derive(Default)]
pub struct LatestSeq {
    home: AtomicU64,
    explore: AtomicU64,
    search: AtomicU64,
    suggest: AtomicU64,
    open: AtomicU64,
}

impl LatestSeq {
    fn slot(&self, job: &RemoteJob) -> Option<(&AtomicU64, u64)> {
        match job {
            RemoteJob::LoadHome { seq, .. } => Some((&self.home, *seq)),
            RemoteJob::LoadExplore { seq } => Some((&self.explore, *seq)),
            RemoteJob::Search { seq, .. } => Some((&self.search, *seq)),
            RemoteJob::Suggest { seq, .. } => Some((&self.suggest, *seq)),
            RemoteJob::Open { seq, .. } => Some((&self.open, *seq)),
            _ => None,
        }
    }

    /// Registra o job que está entrando no pool.
    pub fn note(&self, job: &RemoteJob) {
        if let Some((a, seq)) = self.slot(job) {
            a.fetch_max(seq, Ordering::SeqCst);
        }
    }

    /// Há um job mais novo do mesmo tipo?
    pub fn is_stale(&self, job: &RemoteJob) -> bool {
        self.slot(job).is_some_and(|(a, seq)| a.load(Ordering::SeqCst) > seq)
    }
}

/// Fila dos jobs de rede com debounce: a busca e as sugestões de uma rajada de digitação viram uma só (a última),
/// disparada `debounce` depois da última tecla; qualquer outro job passa na hora.
pub struct RemoteQueue {
    rx: Receiver<RemoteJob>,
    debounce: Duration,
    pending: Vec<(RemoteJob, Instant)>,
}

impl RemoteQueue {
    pub fn new(rx: Receiver<RemoteJob>, debounce: Duration) -> Self {
        Self { rx, debounce, pending: Vec::new() }
    }

    fn slot(job: &RemoteJob) -> Option<u8> {
        match job {
            RemoteJob::Search { .. } => Some(0),
            RemoteJob::Suggest { .. } => Some(1),
            _ => None,
        }
    }

    pub fn next_job(&mut self) -> Option<RemoteJob> {
        loop {
            let due = self.pending.iter().enumerate().min_by_key(|(_, (_, at))| *at).map(|(i, (_, at))| (i, *at));
            let received = match due {
                Some((i, at)) => match self.rx.recv_timeout(at.saturating_duration_since(Instant::now())) {
                    Ok(j) => j,
                    Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => return Some(self.pending.remove(i).0),
                },
                None => self.rx.recv().ok()?,
            };
            match Self::slot(&received) {
                Some(slot) => {
                    self.pending.retain(|(j, _)| Self::slot(j) != Some(slot));
                    self.pending.push((received, Instant::now() + self.debounce));
                }
                None => return Some(received),
            }
        }
    }
}

/// Fila de jobs da thread de trabalho, com debounce só para buscas: a última busca de uma rajada roda `debounce`
/// depois da última digitação; qualquer outro job (tocar, capa, curtir) é atendido na hora, sem esperar a busca.
pub struct JobQueue {
    rx: Receiver<Job>,
    debounce: Duration,
    pending_search: Option<(Job, Instant)>,
}

impl JobQueue {
    pub fn new(rx: Receiver<Job>, debounce: Duration) -> Self {
        Self { rx, debounce, pending_search: None }
    }

    /// Próximo job a executar; `None` quando o canal fecha e não há busca pendente.
    pub fn next_job(&mut self) -> Option<Job> {
        loop {
            let received = match &self.pending_search {
                Some((_, deadline)) => match self.rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                    Ok(j) => j,
                    Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => return self.pending_search.take().map(|p| p.0),
                },
                None => self.rx.recv().ok()?,
            };
            match received {
                j @ Job::Search { .. } => self.pending_search = Some((j, Instant::now() + self.debounce)),
                other => return Some(other),
            }
        }
    }
}


fn list_folders(path: &Path, root: &Path) -> Result<FolderListingVm, String> {
    let root = root.canonicalize().map_err(|_| "O armazenamento compartilhado não está acessível.")?;
    let path = path.canonicalize().map_err(|_| "A pasta não está acessível.")?;
    if !path.starts_with(&root) {
        return Err("Escolha uma pasta no armazenamento compartilhado.".into());
    }
    let mut children = std::fs::read_dir(&path).map_err(|_| "O Android não permitiu listar esta pasta.")?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| {
            Some((entry.file_name().to_str()?.to_owned(), entry.path().to_str()?.to_owned()))
        })
        .collect::<Vec<_>>();
    children.sort_by_cached_key(|(name, _)| name.to_lowercase());
    let parent = if path == root { String::new() } else { path.parent().and_then(Path::to_str).unwrap_or_default().to_owned() };
    Ok(FolderListingVm { current: path.to_string_lossy().into_owned(), parent, children })
}
