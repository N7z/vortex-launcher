// SPDX-License-Identifier: AGPL-3.0-or-later
//! The desktop entry that makes this launcher the vortex:// handler.

use std::path::PathBuf;

use anyhow::{Context, Result};

const FILE: &str = "vortex-launcher.desktop";
const HANDLER_FILE: &str = "vortex-launcher-uri.desktop";
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

/// a link from the browser needs no window, so the headless binary handles it when
/// it sits next to us; falls back to the given exe when only the GUI is installed
fn handler_exe(exe: &str) -> String {
    let cli = PathBuf::from(exe).with_file_name("vortex-launcher-cli");
    if cli.is_file() {
        return cli.display().to_string();
    }
    exe.to_owned()
}

/// the menu entry: always the GUI, and it does not claim the scheme
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
         StartupWMClass=vortex-launcher\n"
    )
}

/// the vortex:// handler, hidden from the menu so only the browser reaches it
fn handler_entry(exe: &str) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Vortex Launcher (link handler)\n\
         Comment=Opens vortex:// links without a window\n\
         Exec={exe} %u\n\
         Icon={ICON}\n\
         Terminal=false\n\
         NoDisplay=true\n\
         Categories=Game;\n\
         MimeType=x-scheme-handler/vortex;\n",
        exe = handler_exe(exe)
    )
}

/// writes the entries and claims the scheme, only when something actually changed
pub fn install() -> Result<bool> {
    let dir = applications_dir().context("no XDG data directory")?;
    let exe = std::env::current_exe()
        .context("cannot find our own path")?
        .display()
        .to_string();
    install_icon()?;

    let files = [(FILE, entry(&exe)), (HANDLER_FILE, handler_entry(&exe))];
    let mut changed = false;
    for (name, wanted) in &files {
        let path = dir.join(name);
        if std::fs::read_to_string(&path).is_ok_and(|found| &found == wanted) {
            continue;
        }
        std::fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
        std::fs::write(&path, wanted).with_context(|| format!("cannot write {}", path.display()))?;
        changed = true;
    }
    if !changed {
        return Ok(false);
    }

    run("update-desktop-database", &[dir.to_string_lossy().as_ref()]);
    run("xdg-mime", &["default", HANDLER_FILE, "x-scheme-handler/vortex"]);
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
