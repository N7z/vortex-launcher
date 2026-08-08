// SPDX-License-Identifier: AGPL-3.0-or-later
//! Persisted launcher state. Written by the GUI, never meant to be hand-edited.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub game: Option<GameInstall>,
    pub studio: Option<GameInstall>,
    pub proton: Option<ProtonInstall>,
    /// a build the user picked from the release list, downloaded on the next install
    pub proton_wanted: Option<String>,
    /// on by default, a pinned old client cannot join a patched server
    pub allow_self_update: bool,
    /// off by default: it only helps the GPUs where vkd3d-shader miscompiles,
    /// and turning it on for everyone would change a rendering path that works
    pub native_shader_compiler: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GameInstall {
    pub exe: PathBuf,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub size: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProtonInstall {
    pub name: String,
    /// directory holding the proton script
    pub dir: PathBuf,
    pub source: ProtonSource,
}

/// ordered: a managed build is preferred over one borrowed from steam
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProtonSource {
    Managed,
    System,
}

impl Config {
    pub fn load(path: &Path) -> Self {
        // a missing or broken config is not fatal, everything gets re-detected
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_else(|| Self {
                allow_self_update: true,
                ..Self::default()
            })
    }

    /// atomic, so a kill mid-write does not cost a 30 MB re-download
    pub fn save(&self, path: &Path) -> Result<()> {
        let parent = path.parent().context("config path has no parent")?;
        crate::paths::create_dir(parent)?;
        let tmp = path.with_extension("json.tmp");
        let text = serde_json::to_string_pretty(self).context("cannot serialize config")?;
        std::fs::write(&tmp, text).with_context(|| format!("cannot write {}", tmp.display()))?;
        std::fs::rename(&tmp, path).with_context(|| format!("cannot replace {}", path.display()))?;
        Ok(())
    }

    pub fn game_ready(&self) -> bool {
        self.game.as_ref().is_some_and(|g| g.exe.is_file())
    }

    pub fn studio_ready(&self) -> bool {
        self.studio.as_ref().is_some_and(|s| s.exe.is_file())
    }

    pub fn proton_ready(&self) -> bool {
        self.proton
            .as_ref()
            .is_some_and(|p| crate::proton::is_usable(&p.dir))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// configs written before the picker existed must still load
    #[test]
    fn an_old_config_loads_without_the_wanted_field() {
        let dir = std::env::temp_dir().join("vortex-config-compat");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        std::fs::write(&path, br#"{"allow_self_update":true,"native_shader_compiler":false}"#).unwrap();

        let config = Config::load(&path);
        assert!(config.allow_self_update);
        assert!(config.proton_wanted.is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_pending_pick_survives_a_save_and_load() {
        let dir = std::env::temp_dir().join("vortex-config-wanted");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");

        let mut config = Config::default();
        config.proton_wanted = Some("GE-Proton9-27".into());
        config.save(&path).unwrap();

        let loaded = Config::load(&path);
        assert_eq!(loaded.proton_wanted.as_deref(), Some("GE-Proton9-27"));
        // nothing on disk yet, so the window falls back to the install screen
        assert!(!loaded.proton_ready());
        std::fs::remove_dir_all(&dir).ok();
    }
}
