// SPDX-License-Identifier: AGPL-3.0-or-later
//! Every path the launcher touches, all under the XDG base dirs.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

const APP: &str = "vortex-launcher";

#[derive(Clone, Debug)]
pub struct Paths {
    pub data: PathBuf,
    pub config: PathBuf,
}

impl Paths {
    pub fn discover() -> Result<Self> {
        let data = dirs::data_dir()
            .context("no XDG data directory (is $HOME set?)")?
            .join(APP);
        let config = dirs::config_dir()
            .context("no XDG config directory (is $HOME set?)")?
            .join(APP);
        Ok(Self { data, config })
    }

    /// holds Vortex.exe
    pub fn game(&self) -> PathBuf {
        self.data.join("game")
    }

    /// one directory per proton build
    pub fn proton(&self) -> PathBuf {
        self.data.join("proton")
    }

    /// STEAM_COMPAT_DATA_PATH, proton puts the prefix in pfx/ below it
    pub fn prefix(&self) -> PathBuf {
        self.data.join("prefix")
    }

    /// STEAM_COMPAT_CLIENT_INSTALL_PATH, proton wants one even without steam
    pub fn compat_client(&self) -> PathBuf {
        self.data.join("compat-client")
    }

    /// partials live here so they can resume across runs
    pub fn downloads(&self) -> PathBuf {
        self.data.join("downloads")
    }

    pub fn logs(&self) -> PathBuf {
        self.data.join("logs")
    }

    pub fn config_file(&self) -> PathBuf {
        self.config.join("config.json")
    }

    pub fn ensure(&self) -> Result<()> {
        for dir in [
            self.data.clone(),
            self.config.clone(),
            self.game(),
            self.proton(),
            self.prefix(),
            self.compat_client(),
            self.downloads(),
            self.logs(),
        ] {
            create_dir(&dir)?;
        }
        Ok(())
    }
}

pub fn create_dir(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("cannot create directory {}", dir.display()))
}

/// free bytes where dir lives, None when we cannot tell
pub fn free_space(dir: &Path) -> Option<u64> {
    let out = std::process::Command::new("df").arg("-Pk").arg(dir).output().ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    let available_kb: u64 = text.lines().nth(1)?.split_whitespace().nth(3)?.parse().ok()?;
    Some(available_kb.saturating_mul(1024))
}
