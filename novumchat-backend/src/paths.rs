//! Platform-appropriate locations for the config file and the database.
//!
//! Uses the `directories` crate (the Rust counterpart of Python's
//! `platformdirs`):
//!
//! | OS      | config.toml                                              | novumchat.db                                           |
//! |---------|----------------------------------------------------------|--------------------------------------------------------|
//! | Linux   | `~/.config/novumchat/`                                   | `~/.local/share/novumchat/`                            |
//! | macOS   | `~/Library/Application Support/com.NovumChat.NovumChat/` | same                                                   |
//! | Windows | `%APPDATA%\NovumChat\NovumChat\config\`                  | `%APPDATA%\NovumChat\NovumChat\data\`                  |
//!
//! On Android/iOS the sandboxed app directories reported by Tauri are used.
//! `NOVUMCHAT_CONFIG_DIR` / `NOVUMCHAT_DATA_DIR` override both.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
    /// Default working directory for file/shell tools.
    pub default_workspace: PathBuf,
}

impl AppPaths {
    /// Resolves paths. `mobile_fallback` carries Tauri's sandbox directories on
    /// platforms where `directories` has no answer.
    pub fn resolve(mobile_fallback: Option<AppPaths>) -> Result<Self> {
        let env_config = std::env::var_os("NOVUMCHAT_CONFIG_DIR").map(PathBuf::from);
        let env_data = std::env::var_os("NOVUMCHAT_DATA_DIR").map(PathBuf::from);

        let (mut config_dir, mut data_dir, mut cache_dir, home) = if let Some(fallback) = mobile_fallback {
            (fallback.config_dir, fallback.data_dir, fallback.cache_dir, Some(fallback.default_workspace))
        } else {
            let dirs = directories::ProjectDirs::from("com", "NovumChat", "NovumChat")
                .context("could not determine the platform directories for NovumChat")?;
            let home = directories::UserDirs::new().map(|u| u.home_dir().join("NovumChat"));
            (dirs.config_dir().to_path_buf(), dirs.data_dir().to_path_buf(), dirs.cache_dir().to_path_buf(), home)
        };
        if let Some(c) = env_config {
            config_dir = c;
        }
        if let Some(d) = env_data {
            cache_dir = d.join("cache");
            data_dir = d;
        }
        let default_workspace = home.unwrap_or_else(|| data_dir.join("workspace"));
        let paths = Self { config_dir, data_dir, cache_dir, default_workspace };
        paths.ensure()?;
        Ok(paths)
    }

    fn ensure(&self) -> Result<()> {
        for dir in [&self.config_dir, &self.data_dir, &self.cache_dir] {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        Ok(())
    }

    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    pub fn database_file(&self) -> PathBuf {
        self.data_dir.join("novumchat.db")
    }

    /// The workspace for tools: configured value (with `~` expansion) or the default.
    pub fn workspace(&self, configured: Option<&str>) -> PathBuf {
        match configured.map(str::trim).filter(|s| !s.is_empty()) {
            Some(p) => expand_tilde(p),
            None => self.default_workspace.clone(),
        }
    }
}

/// Expands a leading `~` to the home directory.
pub fn expand_tilde(p: &str) -> PathBuf {
    if (p == "~" || p.starts_with("~/") || p.starts_with("~\\"))
        && let Some(u) = directories::UserDirs::new()
    {
        let rest = p[1..].trim_start_matches(['/', '\\']);
        return if rest.is_empty() { u.home_dir().to_path_buf() } else { u.home_dir().join(rest) };
    }
    PathBuf::from(p)
}

/// Lexically normalises `path` (resolving `.` and `..`) without touching the file system.
pub fn normalize_lexically(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::ParentDir => {
                if out.as_os_str().is_empty() || out.ends_with("..") {
                    out.push("..");
                } else {
                    // At the root `pop` fails and we stay there.
                    out.pop();
                }
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexical_normalization() {
        assert_eq!(normalize_lexically(Path::new("/a/b/../c/./d")), PathBuf::from("/a/c/d"));
        assert_eq!(normalize_lexically(Path::new("/a/../../b")), PathBuf::from("/b"));
    }

    #[test]
    fn tilde() {
        let p = expand_tilde("~/x");
        assert!(p.ends_with("x"));
        assert!(!p.to_string_lossy().starts_with('~'));
        assert_eq!(expand_tilde("/abs"), PathBuf::from("/abs"));
    }
}
