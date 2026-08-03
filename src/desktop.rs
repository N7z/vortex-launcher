// SPDX-License-Identifier: AGPL-3.0-or-later
//! The desktop entry that makes this launcher the vortex:// handler.

use std::path::PathBuf;

use anyhow::{Context, Result};

const FILE: &str = "vortex-launcher.desktop";
const ICON: &str = "vortex-launcher";

fn applications_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|dir| dir.join("applications"))
}

/// the icon has to live in the hicolor theme for Icon= to resolve by name
fn install_icon() -> Result<()> {
    let dir = dirs::data_dir()
        .context("no XDG data directory")?
        .join("icons/hicolor/256x256/apps");
    let path = dir.join(format!("{ICON}.png"));
    let wanted = crate::logo::png_bytes();

    if std::fs::read(&path).is_ok_and(|found| found == wanted) {
        return Ok(());
    }
    std::fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
    std::fs::write(&path, wanted).with_context(|| format!("cannot write {}", path.display()))?;

    if let Some(theme) = dir.parent().and_then(|d| d.parent()) {
        run("gtk-update-icon-cache", &["-t", "-q", theme.to_string_lossy().as_ref()]);
    }
    Ok(())
}

fn entry(exe: &str) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Vortex Launcher\n\
         Comment=Unofficial Linux launcher for Vortex\n\
         Exec={exe} %u\n\
         Icon={ICON}\n\
         Terminal=false\n\
         Categories=Game;\n\
         Keywords=vortex;\n\
         StartupWMClass=vortex-launcher\n\
         MimeType=x-scheme-handler/vortex;\n"
    )
}

/// writes the entry and claims the scheme, only when something actually changed
pub fn install() -> Result<bool> {
    let dir = applications_dir().context("no XDG data directory")?;
    let exe = std::env::current_exe()
        .context("cannot find our own path")?
        .display()
        .to_string();
    let wanted = entry(&exe);
    let path = dir.join(FILE);
    install_icon()?;

    if std::fs::read_to_string(&path).is_ok_and(|found| found == wanted) {
        return Ok(false);
    }

    std::fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
    std::fs::write(&path, &wanted).with_context(|| format!("cannot write {}", path.display()))?;

    run("update-desktop-database", &[dir.to_string_lossy().as_ref()]);
    run("xdg-mime", &["default", FILE, "x-scheme-handler/vortex"]);
    Ok(true)
}

fn run(program: &str, args: &[&str]) {
    std::process::Command::new(program)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .ok();
}
