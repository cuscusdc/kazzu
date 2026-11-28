//! # kazzu-library
//!
//! Biblioteca local do Kazzu: scanner de pastas de música, tags (lofty), capas
//! em cache (com versão pré-desfocada para o fundo glass), banco SQLite com
//! migrações versionadas e FTS5, playlists, curtidos e histórico de reprodução.
//!
//! Tudo é local: **nenhum dado sai da máquina** (o histórico existe apenas para
//! a futura retrospectiva).
//!
//! API síncrona. [`Library`] é `Send` (mas não `Sync`): use uma instância por
//! thread/tarefa, ou proteja com `Mutex`.
//!
//! ```no_run
//! use kazzu_library::{Library, ScanProgress};
//! # fn main() -> kazzu_library::Result<()> {
//! let mut lib = Library::open_default()?;
//! lib.add_folder("/home/user/Music")?;
//! lib.scan(&|p| if let ScanProgress::Reading { done, total } = p { println!("{done}/{total}") })?;
//! for album in lib.albums()? {
//!     println!("{} — {}", album.artist, album.title);
//! }
//! # Ok(()) }
//! ```

mod artwork;
mod core_conv;
mod db;
mod error;
mod model;
mod queries;
mod scan;
mod scanner;
mod statistics;
mod tag_edit;

use std::path::{Path, PathBuf};

use rusqlite::Connection;

pub use error::{Error, Result};
pub use model::{
    Album, AlbumId, Artist, ArtistId, Artwork, HistoryEntry, Playlist, PlaylistEntry, PlaylistId,
    Rgb, ScanProgress, ScanReport, SearchResults, Track, TrackId,
};

/// Extensões de áudio reconhecidas pelo scanner.
pub use scanner::AUDIO_EXTENSIONS;
pub use queries::{RestorePlaylistEntry, RestoredPlaylist};
pub use statistics::{ListeningRank, ListeningStats};
pub use tag_edit::update_tags;

/// Handle da biblioteca local.
pub struct Library {
    pub(crate) conn: Connection,
    pub(crate) cache_dir: PathBuf,
}

impl Library {
    /// Abre (ou cria) a biblioteca em `data_dir`:
    /// `data_dir/library.sqlite` e `data_dir/cache/covers/`.
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self> {
        let data_dir = data_dir.as_ref();
        std::fs::create_dir_all(data_dir).map_err(|e| Error::io(data_dir, e))?;
        Self::open_with(data_dir.join("library.sqlite"), data_dir.join("cache"))
    }

    /// Abre usando o diretório de dados XDG do usuário
    /// (`~/.local/share/kazzu` no Linux, equivalentes nos demais SOs).
    pub fn open_default() -> Result<Self> {
        Self::open(Self::default_data_dir()?)
    }

    /// Diretório de dados padrão (via `directories`).
    pub fn default_data_dir() -> Result<PathBuf> {
        directories::ProjectDirs::from("app", "kazzu", "Kazzu")
            .map(|d| d.data_dir().to_path_buf())
            .ok_or(Error::NoDataDir)
    }

    /// Caminhos do banco e do cache escolhidos separadamente.
    pub fn open_with(db_path: impl AsRef<Path>, cache_dir: impl Into<PathBuf>) -> Result<Self> {
        let conn = Connection::open(db_path.as_ref())?;
        Self::from_connection(conn, cache_dir.into())
    }

    /// Banco em memória (testes); o cache de capas continua em disco.
    pub fn open_in_memory(cache_dir: impl Into<PathBuf>) -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?, cache_dir.into())
    }

    fn from_connection(mut conn: Connection, cache_dir: PathBuf) -> Result<Self> {
        db::configure(&conn)?;
        db::migrate(&mut conn)?;
        Ok(Self { conn, cache_dir })
    }

    /// Diretório do cache (capas etc.).
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// Versão do esquema do banco.
    pub fn schema_version(&self) -> Result<i64> {
        Ok(self
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))?)
    }
}

pub(crate) fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}
