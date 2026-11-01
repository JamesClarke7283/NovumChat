//! Loading and saving `config.toml`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use novumchat_core::config::{AppConfig, DEFAULT_CONFIG_TOML};
use parking_lot::RwLock;

pub struct ConfigStore {
    path: PathBuf,
    current: RwLock<AppConfig>,
    /// Problem found while loading (shown once in the UI).
    load_error: RwLock<Option<String>>,
}

const HEADER: &str = "# NovumChat configuration — written by the app. Hand edits are welcome;\n\
# comments are kept only when you edit the file through Settings → Advanced.\n\n";

impl ConfigStore {
    /// Loads the config, writing the commented default template on first launch.
    /// A malformed file is never overwritten: defaults are used in memory and
    /// the error is reported.
    pub fn load(path: PathBuf) -> Result<Self> {
        if !path.exists() {
            write_private(&path, DEFAULT_CONFIG_TOML)
                .with_context(|| format!("writing default config to {}", path.display()))?;
        }
        let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        let (config, load_error) = match toml::from_str::<AppConfig>(&text) {
            Ok(mut cfg) => {
                let problems = cfg.normalize();
                (cfg, (!problems.is_empty()).then(|| problems.join("; ")))
            }
            Err(e) => {
                tracing::error!("config.toml is invalid, using defaults: {e}");
                (AppConfig::default(), Some(format!("config.toml could not be parsed, using defaults: {e}")))
            }
        };
        Ok(Self { path, current: RwLock::new(config), load_error: RwLock::new(load_error) })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn get(&self) -> AppConfig {
        self.current.read().clone()
    }

    /// Runs `f` with a borrowed config (avoids cloning on hot paths).
    pub fn with<R>(&self, f: impl FnOnce(&AppConfig) -> R) -> R {
        f(&self.current.read())
    }

    pub fn take_load_error(&self) -> Option<String> {
        self.load_error.write().take()
    }

    /// Validates, normalises and persists a new config.
    pub fn save(&self, mut config: AppConfig) -> Result<AppConfig> {
        config.normalize();
        let body = toml::to_string_pretty(&config).context("serialising config")?;
        write_private(&self.path, &format!("{HEADER}{body}"))?;
        *self.current.write() = config.clone();
        Ok(config)
    }

    /// The file exactly as it is on disk (for the raw editor).
    pub fn raw(&self) -> Result<String> {
        std::fs::read_to_string(&self.path).with_context(|| format!("reading {}", self.path.display()))
    }

    /// Saves hand-written TOML verbatim after validating it.
    pub fn save_raw(&self, text: &str) -> Result<AppConfig> {
        let mut config: AppConfig = toml::from_str(text).context("invalid TOML")?;
        config.normalize();
        write_private(&self.path, text)?;
        *self.current.write() = config.clone();
        Ok(config)
    }
}

/// Atomically writes `contents`, readable only by the current user on Unix
/// (the file may contain API keys).
pub fn write_private(path: &Path, contents: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, contents).with_context(|| format!("writing {}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_launch_writes_template_and_saves_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let store = ConfigStore::load(path.clone()).unwrap();
        assert!(path.exists());
        assert!(std::fs::read_to_string(&path).unwrap().contains("[[providers]]"));
        assert!(store.take_load_error().is_none());

        let mut cfg = store.get();
        cfg.appearance.theme = novumchat_core::config::ThemeMode::Dark;
        store.save(cfg).unwrap();
        let reloaded = ConfigStore::load(path.clone()).unwrap();
        assert_eq!(reloaded.get().appearance.theme, novumchat_core::config::ThemeMode::Dark);
    }

    #[test]
    fn invalid_file_is_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "this is = = not toml").unwrap();
        let store = ConfigStore::load(path.clone()).unwrap();
        assert!(store.take_load_error().is_some());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "this is = = not toml");
    }
}
