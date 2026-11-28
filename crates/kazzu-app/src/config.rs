//! Configuração do app (TOML em `~/.config/kazzu/config.toml` no Linux, via `directories`).
//!
//! As pastas de música ficam só na biblioteca (SQLite): é a fonte única, o scan já as lê de lá.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Tema escolhido pelo usuário.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemePref {
    /// Segue o sistema.
    #[default]
    Auto,
    Light,
    Dark,
    Oled,
}

/// Origem da cor de destaque.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccentPref {
    /// Derivada da capa da faixa atual.
    #[default]
    Cover,
    /// Âmbar da marca, fixo.
    Brand,
}

/// Qualidade pedida aos provedores de stream (Ajustes › Áudio).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamQualityPref {
    /// A melhor que o provedor oferece, sem transformar.
    #[default]
    Original,
    /// Melhor lossy (economiza dados).
    High,
    /// Pouca banda.
    Low,
}

impl StreamQualityPref {
    pub fn from_index(i: i32) -> Self {
        match i {
            1 => Self::High,
            2 => Self::Low,
            _ => Self::Original,
        }
    }

    pub fn index(self) -> i32 {
        match self {
            Self::Original => 0,
            Self::High => 1,
            Self::Low => 2,
        }
    }

    pub fn to_core(self) -> kazzu_core::StreamQuality {
        match self {
            Self::Original => kazzu_core::StreamQuality::Original,
            Self::High => kazzu_core::StreamQuality::High,
            Self::Low => kazzu_core::StreamQuality::Low,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MobileAppearance {
    pub glass: [f32; 5],
    pub rounded: bool,
    pub effects: i32,
    pub vinyl: i32,
    pub crossfade_seconds: i32,
}

impl Default for MobileAppearance {
    fn default() -> Self {
        Self { glass: [45.0, 24.0, 48.0, 107.0, 36.0], rounded: true, effects: 1, vinyl: 0, crossfade_seconds: 0 }
    }
}

impl MobileAppearance {
    pub fn sanitize(&mut self) {
        let ranges = [(0.0, 100.0), (0.0, 40.0), (0.0, 100.0), (80.0, 130.0), (0.0, 100.0)];
        let defaults = Self::default().glass;
        for (i, (min, max)) in ranges.into_iter().enumerate() {
            let value = self.glass[i];
            self.glass[i] = if value.is_finite() { value.clamp(min, max) } else { defaults[i] };
        }
        self.effects = self.effects.clamp(0, 2);
        self.vinyl = self.vinyl.clamp(0, 2);
        self.crossfade_seconds = self.crossfade_seconds.clamp(0, 8) / 2 * 2;
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub glass_style: i32,
    pub color_palette: i32,
    pub artwork_background: bool,
    pub library_view: i32,
    pub library_density: i32,
    pub theme: ThemePref,
    pub accent: AccentPref,
    /// `None` = ainda não decidido (usa a preferência do sistema, se houver).
    pub reduce_motion: Option<bool>,
    pub reduce_transparency: Option<bool>,
    /// Volume linear 0..=1.
    pub volume: f32,
    /// Buscas recentes (mais nova primeiro), só neste aparelho.
    pub recent_searches: Vec<String>,
    /// Qualidade dos streams remotos.
    pub stream_quality: StreamQualityPref,
    /// Quando a fila de uma fonte remota acaba, continuar com músicas parecidas (rádio).
    pub autoplay_radio: bool,
    /// O aviso "O YouTube Music já está ativo" já foi dispensado (nunca volta).
    pub first_run_dismissed: bool,
    pub mobile_appearance: MobileAppearance,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            glass_style: 1,
            color_palette: 0,
            artwork_background: true,
            library_view: 0,
            library_density: 1,
            theme: ThemePref::Dark,
            accent: AccentPref::Cover,
            reduce_motion: None,
            reduce_transparency: None,
            volume: 1.0,
            recent_searches: Vec::new(),
            stream_quality: StreamQualityPref::Original,
            autoplay_radio: true,
            first_run_dismissed: false,
            mobile_appearance: MobileAppearance::default(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("não foi possível ler {0}: {1}")]
    Read(PathBuf, std::io::Error),
    #[error("configuração inválida em {0}: {1}")]
    Parse(PathBuf, toml::de::Error),
    #[error("não foi possível salvar {0}: {1}")]
    Write(PathBuf, std::io::Error),
    #[error("não foi possível serializar a configuração: {0}")]
    Serialize(toml::ser::Error),
}

impl Config {
    /// Número máximo de buscas recentes guardadas.
    pub const MAX_RECENT: usize = 8;

    /// Caminho padrão do arquivo de configuração.
    pub fn default_path() -> Option<PathBuf> {
        directories::ProjectDirs::from("app", "kazzu", "Kazzu").map(|d| d.config_dir().join("config.toml"))
    }

    /// Lê o arquivo. Arquivo ausente = configuração padrão (não é erro).
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str::<Config>(&text).map(Config::sanitized).map_err(|e| ConfigError::Parse(path.to_path_buf(), e)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(ConfigError::Read(path.to_path_buf(), e)),
        }
    }

    /// Corrige valores impossíveis vindos de um arquivo editado à mão (`volume = nan`, 7, -1...).
    pub fn sanitized(mut self) -> Self {
        self.volume = if self.volume.is_finite() { self.volume.clamp(0.0, 1.0) } else { 1.0 };
        self.glass_style = self.glass_style.clamp(0, 3);
        self.color_palette = self.color_palette.clamp(0, 5);
        self.library_view = self.library_view.clamp(0, 1);
        self.library_density = self.library_density.clamp(0, 2);
        self.recent_searches.truncate(Self::MAX_RECENT);
        self.mobile_appearance.sanitize();
        self
    }

    /// Move um arquivo ilegível para `<nome>.bak` (sobrescrevendo um `.bak` antigo) para que o app possa gravar um novo
    /// sem perder o que o usuário tinha escrito. Devolve o caminho do backup.
    pub fn back_up_corrupt(path: &Path) -> Result<PathBuf, ConfigError> {
        let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
        name.push(".bak");
        let bak = path.with_file_name(name);
        std::fs::rename(path, &bak).map_err(|e| ConfigError::Write(bak.clone(), e))?;
        Ok(bak)
    }

    /// Salva de forma atômica (arquivo temporário + rename).
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        let text = toml::to_string_pretty(self).map_err(ConfigError::Serialize)?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| ConfigError::Write(dir.to_path_buf(), e))?;
        }
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text).map_err(|e| ConfigError::Write(tmp.clone(), e))?;
        std::fs::rename(&tmp, path).map_err(|e| ConfigError::Write(path.to_path_buf(), e))
    }

    /// Registra uma busca recente (sem repetir, no topo, limitada).
    pub fn push_recent_search(&mut self, query: &str) {
        let q = query.trim();
        if q.is_empty() {
            return;
        }
        self.recent_searches.retain(|s| !s.eq_ignore_ascii_case(q));
        self.recent_searches.insert(0, q.to_owned());
        self.recent_searches.truncate(Self::MAX_RECENT);
        self.mobile_appearance.sanitize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_gives_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config::load(&dir.path().join("nope.toml")).unwrap();
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn roundtrip_and_partial_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/config.toml");
        let mut cfg = Config { theme: ThemePref::Light, color_palette: 4, reduce_motion: Some(true), ..Config::default() };
        cfg.push_recent_search("aurora");
        cfg.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap(), cfg);

        // arquivo parcial: campos ausentes assumem o padrão
        std::fs::write(&path, "theme = \"dark\"\n").unwrap();
        let loaded = Config::load(&path).unwrap();
        assert_eq!(loaded.theme, ThemePref::Dark);
        assert_eq!(loaded.volume, 1.0);
    }

    #[test]
    fn invalid_file_is_an_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "theme = [").unwrap();
        assert!(matches!(Config::load(&path), Err(ConfigError::Parse(..))));
    }

    #[test]
    fn absurd_volumes_are_sanitized() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        for (text, want) in [("volume = nan", 1.0), ("volume = inf", 1.0), ("volume = 7.0", 1.0), ("volume = -3.0", 0.0), ("volume = 0.4", 0.4)] {
            std::fs::write(&path, text).unwrap();
            assert_eq!(Config::load(&path).unwrap().volume, want, "{text}");
        }
    }

    #[test]
    fn corrupt_file_is_backed_up_not_lost() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "theme = [").unwrap();
        let bak = Config::back_up_corrupt(&path).unwrap();
        assert_eq!(bak.file_name().unwrap(), "config.toml.bak");
        assert_eq!(std::fs::read_to_string(&bak).unwrap(), "theme = [");
        assert!(!path.exists());
        // um .bak antigo é substituído
        std::fs::write(&path, "x = [").unwrap();
        Config::back_up_corrupt(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&bak).unwrap(), "x = [");
    }

    #[test]
    fn recent_searches_dedupe_and_cap() {
        let mut cfg = Config::default();
        for i in 0..12 {
            cfg.push_recent_search(&format!("q{i}"));
        }
        cfg.push_recent_search("Q5");
        assert_eq!(cfg.recent_searches.len(), Config::MAX_RECENT);
        assert_eq!(cfg.recent_searches[0], "Q5");
        assert_eq!(cfg.recent_searches.iter().filter(|s| s.eq_ignore_ascii_case("q5")).count(), 1);
        cfg.push_recent_search("   ");
        assert_eq!(cfg.recent_searches[0], "Q5");
    }
}
