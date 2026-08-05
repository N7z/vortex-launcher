// SPDX-License-Identifier: AGPL-3.0-or-later
//! Optional native d3dcompiler_47.
//!
//! Vortex compiles its HLSL at runtime through D3DCompile. Under Proton that
//! lands in vkd3d-shader, which on some GPUs rejects shaders the real compiler
//! accepts ("Invalid shader" in the log). The game then presents swapchain
//! images it never rendered to, which reads as a black screen with working
//! audio. Dropping Microsoft's compiler into the prefix fixes it.
//!
//! We do not ship the DLL: it is not ours to redistribute. winetricks fetches
//! it, so the download stays in the tool that already handles that licensing.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use anyhow::{bail, Context, Result};

use crate::config::Config;
use crate::paths::Paths;
use crate::state::Shared;

const DLL: &str = "d3dcompiler_47";
/// native first, but fall back to builtin so a half-finished install still boots
pub const OVERRIDE: &str = "d3dcompiler_47=n,b";

/// where proton puts the wine prefix below STEAM_COMPAT_DATA_PATH
pub fn prefix(paths: &Paths) -> PathBuf {
    paths.prefix().join("pfx")
}

/// wine stamps this into the PE header of the DLLs it ships itself
const BUILTIN_MARK: &[u8] = b"Wine builtin DLL";

/// the file is always there, proton's own build of it included, so existing is
/// not enough: only a DLL that is *not* wine's own means the override will bite
pub fn is_installed(paths: &Paths) -> bool {
    let dll = prefix(paths)
        .join("drive_c/windows/system32")
        .join(format!("{DLL}.dll"));
    match std::fs::read(&dll) {
        Ok(bytes) => !is_wine_builtin(&bytes),
        Err(_) => false,
    }
}

fn is_wine_builtin(dll: &[u8]) -> bool {
    // the mark sits in the DOS stub, well inside the first block
    dll.get(..4096)
        .unwrap_or(dll)
        .windows(BUILTIN_MARK.len())
        .any(|window| window == BUILTIN_MARK)
}

/// the wine binary belonging to a proton build, so winetricks does not touch
/// the prefix with a different wine version and upgrade it out from under us
fn wine(proton_dir: &Path) -> Option<PathBuf> {
    ["files/bin/wine", "dist/bin/wine"]
        .iter()
        .map(|rel| proton_dir.join(rel))
        .find(|path| path.is_file())
}

fn winetricks() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join("winetricks"))
        .find(|candidate| candidate.is_file())
}

pub fn install(paths: &Paths, config: &Config, shared: &Arc<Shared>) -> Result<()> {
    if is_installed(paths) {
        return Ok(());
    }

    let prefix = prefix(paths);
    if !prefix.is_dir() {
        bail!("the wine prefix does not exist yet, start Vortex once and then turn this on");
    }
    let Some(winetricks) = winetricks() else {
        bail!("winetricks is not installed, install it and try again");
    };
    let proton = config
        .proton
        .as_ref()
        .context("Proton is not installed yet")?;

    shared.log(format!("installing native {DLL} via winetricks"));
    let mut command = Command::new(&winetricks);
    command.arg("-q").arg(DLL).env("WINEPREFIX", &prefix);
    if let Some(wine) = wine(&proton.dir) {
        command.env("WINESERVER", wine.with_file_name("wineserver"));
        command.env("WINE", wine);
    }

    let output = command
        .output()
        .with_context(|| format!("cannot run {}", winetricks.display()))?;
    if !output.status.success() {
        let reason = String::from_utf8_lossy(&output.stderr);
        let reason = reason
            .lines()
            .rev()
            .find(|line| line.chars().any(|c| c.is_alphanumeric()));
        match reason {
            Some(line) => bail!("winetricks could not install {DLL}: {line}"),
            None => bail!("winetricks could not install {DLL}"),
        }
    }

    if !is_installed(paths) {
        bail!("winetricks finished but {DLL}.dll is not in the prefix");
    }
    shared.log(format!("native {DLL} is in place"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wine_own_dll_does_not_count_as_installed() {
        let mut builtin = vec![0u8; 512];
        builtin[64..64 + BUILTIN_MARK.len()].copy_from_slice(BUILTIN_MARK);
        assert!(is_wine_builtin(&builtin));
        assert!(!is_wine_builtin(&vec![0u8; 512]));
    }

    #[test]
    fn prefix_sits_below_compat_data() {
        let paths = Paths {
            data: PathBuf::from("/data"),
            config: PathBuf::from("/config"),
        };
        assert_eq!(prefix(&paths), PathBuf::from("/data/prefix/pfx"));
    }
}
