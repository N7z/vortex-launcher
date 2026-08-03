// SPDX-License-Identifier: AGPL-3.0-or-later
//! Downloading and unpacking the Windows build of Vortex.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::{bail, Context, Result};

use crate::config::GameInstall;
use crate::net::{self, Remote};
use crate::paths::Paths;
use crate::state::Shared;

pub const DOWNLOAD_URL: &str = "https://playvortex.io/download/windows";
const ARCHIVE: &str = "Vortex-Windows.zip";
const EXE: &str = "Vortex.exe";

pub fn probe(agent: &ureq::Agent) -> Result<Remote> {
    net::probe(agent, DOWNLOAD_URL)
}

pub fn install(
    agent: &ureq::Agent,
    paths: &Paths,
    shared: &Arc<Shared>,
    cancel: &AtomicBool,
) -> Result<GameInstall> {
    let remote = probe(agent)?;
    shared.log(format!("vortex build is {}", net::human_bytes(remote.size)));

    // zip plus extracted copy, with room to spare
    let needed = remote.size.saturating_mul(3);
    if let Some(free) = crate::paths::free_space(&paths.data) {
        if remote.size > 0 && free < needed {
            bail!(
                "not enough disk space: {} free, about {} needed",
                net::human_bytes(free),
                net::human_bytes(needed)
            );
        }
    }

    let archive = paths.downloads().join(ARCHIVE);
    shared.update(|status| status.detail = "connecting".into());
    net::download(agent, DOWNLOAD_URL, &archive, cancel, |done, total| {
        report(shared, done, total);
    })?;

    let game_dir = paths.game();
    shared.update(|status| {
        status.progress = None;
        status.detail = "extracting".into();
    });
    if game_dir.exists() {
        std::fs::remove_dir_all(&game_dir)
            .with_context(|| format!("cannot clear {}", game_dir.display()))?;
    }
    crate::paths::create_dir(&game_dir)?;
    extract(&archive, &game_dir, shared)?;

    let exe = find_exe(&game_dir)?;
    std::fs::remove_file(&archive).ok();
    shared.log(format!("installed {}", exe.display()));

    Ok(GameInstall {
        exe,
        etag: remote.etag,
        last_modified: remote.last_modified,
        size: remote.size,
    })
}

fn report(shared: &Arc<Shared>, done: u64, total: u64) {
    shared.update(|status| {
        status.progress = (total > 0).then(|| done as f32 / total as f32);
        status.detail = if total > 0 {
            format!("{} of {}", net::human_bytes(done), net::human_bytes(total))
        } else {
            net::human_bytes(done)
        };
    });
}

fn extract(archive: &Path, dest: &Path, shared: &Arc<Shared>) -> Result<()> {
    let file = std::fs::File::open(archive)
        .with_context(|| format!("cannot open {}", archive.display()))?;
    let mut zip = zip::ZipArchive::new(file)
        .with_context(|| format!("{} is not a valid zip (download corrupt?)", archive.display()))?;

    let count = zip.len();
    for index in 0..count {
        let mut entry = zip.by_index(index).context("cannot read zip entry")?;
        // enclosed_name drops anything trying to escape the destination
        let Some(relative) = entry.enclosed_name() else {
            continue;
        };
        let target = dest.join(relative);

        if entry.is_dir() {
            crate::paths::create_dir(&target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            crate::paths::create_dir(parent)?;
        }
        let mut out = std::fs::File::create(&target)
            .with_context(|| format!("cannot write {}", target.display()))?;
        io::copy(&mut entry, &mut out)
            .with_context(|| format!("cannot unpack {} (disk full?)", target.display()))?;

        if index % 16 == 0 || index + 1 == count {
            let fraction = (index + 1) as f32 / count as f32;
            shared.update(|status| {
                status.progress = Some(fraction);
                status.detail = format!("extracting {} of {count} files", index + 1);
            });
        }
    }
    Ok(())
}

fn find_exe(dir: &Path) -> Result<PathBuf> {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let entries = std::fs::read_dir(&current)
            .with_context(|| format!("cannot read {}", current.display()))?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.eq_ignore_ascii_case(EXE))
            {
                return Ok(path);
            }
        }
    }
    bail!("{EXE} was not in the archive, the download may have changed format")
}
