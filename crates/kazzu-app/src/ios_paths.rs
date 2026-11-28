//! iOS-owned directories for persistent state and disposable cache.

use std::path::PathBuf;

pub struct IosPaths {
    pub files: PathBuf,
    pub cache: PathBuf,
}

impl IosPaths {
    pub fn from_home(home: &std::path::Path) -> Self {
        Self {
            files: home.join("Library").join("Application Support").join("Kazzu"),
            cache: home.join("Library").join("Caches").join("Kazzu"),
        }
    }

    pub fn from_env() -> Self {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        Self::from_home(&home)
    }

    pub fn ensure(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.files)?;
        std::fs::create_dir_all(&self.cache)?;
        Ok(())
    }
}
