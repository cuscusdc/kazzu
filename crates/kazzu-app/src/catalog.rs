//! Provedores remotos como o app os enxerga: o trait [`Catalog`] e o registro [`Providers`].
//!
//! O `kazzu_core::Provider` só tem busca, stream, biblioteca e faixas; a UI do Kazzu também precisa de feed de
//! Início, Explorar, sugestões, página de artista, rádio e curtidas. [`Catalog`] é essa superfície, **síncrona**
//! (roda nas threads de trabalho, nunca na da UI) e pequena o bastante para ser substituída por um falso nos
//! testes (sem rede). [`YtmCatalog`] é a implementação do YouTube Music (`kazzu-ytm`), ativa por padrão.
//!
//! Nenhum método aqui é chamado sem uma ação do usuário ou a abertura do Início (nada de telemetria).

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};

use kazzu_core::{Album, ProviderKind, StreamQuality, StreamSource, Track, TrackId};
use kazzu_ytm::{AlbumPage, ArtistPage, BrowseRef, Category, Config, Discover, Item, PlaylistPage, Radio, SearchFilter, Shelf, ShelfPage, Suggestions, YtMusic};

pub use kazzu_ytm::{ErrorKind, YtmError};

pub type CatResult<T> = Result<T, YtmError>;

/// Uma faixa pronta para o motor: fonte de áudio, faixa completada (duração/formato) e selo de qualidade.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub source: StreamSource,
    pub track: Track,
    /// "OPUS 160 kbps".
    pub quality_label: String,
}

/// O que a tela Explorar mostra.
#[derive(Debug, Clone, Default)]
pub struct ExploreFeed {
    pub new_releases: Vec<Shelf>,
    pub charts: Vec<Shelf>,
    pub categories: Vec<Category>,
}

pub(crate) fn install_resolver(
    engine: kazzu_audio::EngineBuilder,
    catalog: Arc<dyn Catalog>,
    quality: Arc<AtomicI32>,
) -> kazzu_audio::EngineBuilder {
    engine.resolve_streams(move |track| {
        let quality = crate::config::StreamQualityPref::from_index(quality.load(Ordering::Relaxed));
        catalog.resolve(track, quality.to_core()).map(|resolved| resolved.source).map_err(|error| {
            if error.is_retryable() || crate::ytm_vm::is_offline(error.kind()) {
                kazzu_core::Error::Network(error.to_string())
            } else {
                kazzu_core::Error::other(error.to_string())
            }
        })
    })
}

pub trait Catalog: Send + Sync {
    fn kind(&self) -> ProviderKind;

    /// Feed do Início já em cache (sem rede), se houver.
    fn peek_home(&self) -> Option<Vec<Shelf>>;
    fn discover(&self) -> CatResult<Discover>;
    fn explore(&self) -> CatResult<ExploreFeed>;
    fn browse(&self, target: &BrowseRef) -> CatResult<ShelfPage>;
    fn search(&self, text: &str, filter: Option<SearchFilter>, limit: usize) -> CatResult<Vec<Item>>;
    fn suggestions(&self, prefix: &str) -> CatResult<Suggestions>;
    fn album(&self, id: &str) -> CatResult<AlbumPage>;
    fn artist(&self, id: &str) -> CatResult<ArtistPage>;
    fn playlist(&self, id: &str) -> CatResult<PlaylistPage>;
    fn radio(&self, video_id: &str) -> CatResult<Radio>;
    fn lyrics(&self, _video_id: &str) -> CatResult<Option<kazzu_ytm::Lyrics>> { Ok(None) }
    fn radio_continuation(&self, radio: &Radio) -> CatResult<Radio>;

    /// Abre a faixa para o motor (pode demorar: rede). Nunca chamar na thread de UI.
    fn resolve(&self, track: &Track, quality: StreamQuality) -> CatResult<Resolved>;
    /// Esquece a URL guardada (expirou ou a rede mudou) para o próximo `resolve` pedir outra.
    fn invalidate(&self, track: &Track);

    // Biblioteca deste provedor, guardada no aparelho (sem conta).
    fn liked(&self) -> Vec<Track>;
    fn like(&self, track: &Track) -> CatResult<()>;
    fn unlike(&self, id: &TrackId) -> CatResult<bool>;
    fn saved_albums(&self) -> Vec<Album>;
    fn save_album(&self, album: &Album) -> CatResult<()>;
    fn remove_album(&self, id: &str) -> CatResult<bool>;
}

/// Registro dos provedores remotos ativos. O local (arquivos) é o `Store` da biblioteca, não passa por aqui.
#[derive(Clone, Default)]
pub struct Providers {
    remote: HashMap<ProviderKind, Arc<dyn Catalog>>,
    default_remote: Option<ProviderKind>,
}

impl Providers {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registra um provedor; o primeiro registrado vira o padrão.
    pub fn with(mut self, catalog: Arc<dyn Catalog>) -> Self {
        let kind = catalog.kind();
        self.default_remote.get_or_insert(kind);
        self.remote.insert(kind, catalog);
        self
    }

    pub fn get(&self, kind: ProviderKind) -> Option<&Arc<dyn Catalog>> {
        self.remote.get(&kind)
    }

    /// O provedor remoto padrão (YouTube Music): já ativo, sem login.
    pub fn default_remote(&self) -> Option<&Arc<dyn Catalog>> {
        self.default_remote.and_then(|k| self.remote.get(&k))
    }
}

// ---------------------------------------------------------------------------------------------
// YouTube Music
// ---------------------------------------------------------------------------------------------

pub struct YtmCatalog {
    yt: YtMusic,
}

impl YtmCatalog {
    /// Cache de metadados e biblioteca (curtidas) ficam em `data_dir`; o idioma vem do sistema.
    pub fn open(data_dir: &Path) -> CatResult<Self> {
        Ok(Self { yt: YtMusic::new(Config::with_data_dir(data_dir))? })
    }

    /// Keep the provider's persistent library separate from disposable metadata.
    pub fn open_with_cache(data_dir: &Path, cache_dir: &Path) -> CatResult<Self> {
        let mut config = Config::with_data_dir(data_dir);
        config.cache_dir = Some(cache_dir.join("ytm"));
        Ok(Self { yt: YtMusic::new(config)? })
    }
}

impl Catalog for YtmCatalog {
    fn lyrics(&self, video_id: &str) -> CatResult<Option<kazzu_ytm::Lyrics>> {
        self.yt.client().lyrics(video_id)
    }

    fn kind(&self) -> ProviderKind {
        ProviderKind::YtMusic
    }

    fn peek_home(&self) -> Option<Vec<Shelf>> {
        self.yt.client().peek_home().map(|p| p.shelves)
    }

    fn discover(&self) -> CatResult<Discover> {
        self.yt.client().discover()
    }

    fn explore(&self) -> CatResult<ExploreFeed> {
        let c = self.yt.client();
        let releases = c.new_releases();
        let charts = c.charts(None);
        let categories = c.categories();
        if let (Err(e), Err(_), Err(_)) = (&releases, &charts, &categories) {
            return Err(e.clone());
        }
        Ok(ExploreFeed {
            new_releases: releases.map(|p| p.shelves).unwrap_or_default(),
            charts: charts.map(|p| p.shelves).unwrap_or_default(),
            categories: categories.unwrap_or_default(),
        })
    }

    fn browse(&self, target: &BrowseRef) -> CatResult<ShelfPage> {
        self.yt.client().browse(target)
    }

    fn search(&self, text: &str, filter: Option<SearchFilter>, limit: usize) -> CatResult<Vec<Item>> {
        self.yt.client().search_items(text, filter, limit)
    }

    fn suggestions(&self, prefix: &str) -> CatResult<Suggestions> {
        self.yt.client().suggestions(prefix)
    }

    fn album(&self, id: &str) -> CatResult<AlbumPage> {
        self.yt.client().album(id)
    }

    fn artist(&self, id: &str) -> CatResult<ArtistPage> {
        self.yt.client().artist(id)
    }

    fn playlist(&self, id: &str) -> CatResult<PlaylistPage> {
        self.yt.client().playlist(id)
    }

    fn radio(&self, video_id: &str) -> CatResult<Radio> {
        self.yt.client().radio(video_id)
    }

    fn radio_continuation(&self, radio: &Radio) -> CatResult<Radio> {
        self.yt.client().radio_continuation(radio)
    }

    fn resolve(&self, track: &Track, quality: StreamQuality) -> CatResult<Resolved> {
        let id = kazzu_ytm::video_id_of(track).map_err(|e| YtmError::NotFound(e.to_string()))?;
        let stream = self.yt.client().resolve(id, quality)?;
        let mut track = track.clone();
        stream.fill_track(&mut track);
        Ok(Resolved { source: stream.to_stream_source(), quality_label: crate::format::quality_label(&stream.format), track })
    }

    fn invalidate(&self, track: &Track) {
        if let Ok(id) = kazzu_ytm::video_id_of(track) {
            self.yt.invalidate_stream(id);
        }
    }

    fn liked(&self) -> Vec<Track> {
        self.yt.library_store().liked()
    }

    fn like(&self, track: &Track) -> CatResult<()> {
        self.yt.library_store().like(track)
    }

    fn unlike(&self, id: &TrackId) -> CatResult<bool> {
        self.yt.library_store().unlike(id)
    }

    fn saved_albums(&self) -> Vec<Album> {
        self.yt.library_store().snapshot().albums
    }

    fn save_album(&self, album: &Album) -> CatResult<()> {
        self.yt.library_store().save_album(album)
    }

    fn remove_album(&self, id: &str) -> CatResult<bool> {
        self.yt.library_store().remove_album(&kazzu_core::AlbumId::new(id))
    }
}
