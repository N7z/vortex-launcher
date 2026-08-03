// SPDX-License-Identifier: AGPL-3.0-or-later
//! Finding an existing Proton, or fetching GE-Proton ourselves.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha512};

use crate::config::{ProtonInstall, ProtonSource};
use crate::net;
use crate::paths::Paths;
use crate::state::Shared;

const LATEST_RELEASE: &str = "https://api.github.com/repos/GloriousEggroll/proton-ge-custom/releases/latest";

/// where steam keeps proton builds, newest wins
fn system_candidates() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = dirs::home_dir() {
        for base in [
            home.join(".steam/steam"),
            home.join(".steam/root"),
            home.join(".local/share/Steam"),
            home.join(".var/app/com.valvesoftware.Steam/data/Steam"),
        ] {
            dirs.push(base.join("compatibilitytools.d"));
            dirs.push(base.join("steamapps/common"));
        }
    }
    dirs.push(PathBuf::from("/usr/share/steam/compatibilitytools.d"));
    dirs
}

/// e_machine of the host, so an aarch64 build never gets picked on x86_64
fn host_machine() -> Option<u16> {
    match std::env::consts::ARCH {
        "x86_64" => Some(0x3e),
        "aarch64" => Some(0xb7),
        _ => None,
    }
}

/// a proton dir is only usable if its wine actually runs on this cpu
pub fn is_usable(dir: &Path) -> bool {
    if !dir.join("proton").is_file() {
        return false;
    }
    let Some(machine) = host_machine() else {
        return true;
    };
    ["files/bin/wine", "files/bin/wine64", "files/bin/wineserver"]
        .iter()
        .map(|rel| dir.join(rel))
        .find(|path| path.is_file())
        .is_none_or(|path| elf_machine(&path).is_none_or(|found| found == machine))
}

/// e_machine out of the ELF header, None when the file is not an ELF
fn elf_machine(path: &Path) -> Option<u16> {
    use std::io::Read;

    let mut header = [0u8; 20];
    std::fs::File::open(path).ok()?.read_exact(&mut header).ok()?;
    if &header[..4] != b"\x7fELF" {
        return None;
    }
    Some(u16::from_le_bytes([header[18], header[19]]))
}

fn builds_in(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| is_usable(path))
        .filter_map(|path| {
            let name = path.file_name()?.to_string_lossy().into_owned();
            Some((name, path))
        })
        .collect()
}

pub fn detect_system() -> Option<ProtonInstall> {
    let mut found: Vec<(String, PathBuf)> = system_candidates().iter().flat_map(|d| builds_in(d)).collect();
    // prefer GE builds, then the highest name, which sorts newest last in practice
    found.sort_by(|a, b| {
        let ge = |n: &str| n.starts_with("GE-Proton");
        ge(&a.0).cmp(&ge(&b.0)).then_with(|| a.0.cmp(&b.0))
    });
    found.pop().map(|(name, dir)| ProtonInstall {
        name,
        dir,
        source: ProtonSource::System,
    })
}

pub fn detect_managed(paths: &Paths) -> Option<ProtonInstall> {
    let mut builds = builds_in(&paths.proton());
    builds.sort();
    builds.pop().map(|(name, dir)| ProtonInstall {
        name,
        dir,
        source: ProtonSource::Managed,
    })
}

struct Release {
    tag: String,
    tarball: String,
    checksum: Option<String>,
}

/// releases ship x86_64 and aarch64 side by side, and aarch64 sorts first
fn tarball_for_host(names: &[&str], arch: &str) -> Option<String> {
    let wants_arm = arch == "aarch64";
    names
        .iter()
        .find(|name| {
            let arm = name.contains("aarch64") || name.contains("arm64");
            name.ends_with(".tar.gz") && arm == wants_arm
        })
        .map(|name| (*name).to_owned())
}

fn checksum_for(tarball: &str) -> String {
    format!("{}.sha512sum", tarball.trim_end_matches(".tar.gz"))
}

fn latest_release(agent: &ureq::Agent) -> Result<Release> {
    let body = net::get_text(agent, LATEST_RELEASE).context("cannot ask GitHub for GE-Proton")?;
    let json: serde_json::Value =
        serde_json::from_str(&body).context("GitHub sent something that is not JSON")?;

    let tag = json["tag_name"]
        .as_str()
        .context("GitHub release has no tag")?
        .to_owned();
    let assets = json["assets"].as_array().context("GitHub release has no assets")?;

    let names: Vec<&str> = assets.iter().filter_map(|a| a["name"].as_str()).collect();
    let wanted = tarball_for_host(&names, std::env::consts::ARCH).with_context(|| {
        format!("no GE-Proton build for {} in {tag}", std::env::consts::ARCH)
    })?;
    let checksum_name = checksum_for(&wanted);

    let url_of = |name: &str| {
        assets
            .iter()
            .find(|a| a["name"].as_str() == Some(name))
            .and_then(|a| a["browser_download_url"].as_str())
            .map(str::to_owned)
    };

    let tarball = url_of(&wanted).context("GitHub asset has no download url")?;
    Ok(Release {
        tag,
        tarball,
        checksum: url_of(&checksum_name),
    })
}

pub fn install(
    agent: &ureq::Agent,
    paths: &Paths,
    shared: &Arc<Shared>,
    cancel: &AtomicBool,
) -> Result<ProtonInstall> {
    let release = latest_release(agent)?;
    shared.log(format!("latest proton is {}", release.tag));

    if let Some(free) = crate::paths::free_space(&paths.data) {
        const NEEDED: u64 = 2 * 1024 * 1024 * 1024;
        if free < NEEDED {
            bail!(
                "not enough disk space for Proton: {} free, about {} needed",
                net::human_bytes(free),
                net::human_bytes(NEEDED)
            );
        }
    }

    let tarball = paths.downloads().join(format!("{}.tar.gz", release.tag));
    shared.update(|status| status.detail = "connecting".into());
    net::download(agent, &release.tarball, &tarball, cancel, |done, total| {
        shared.update(|status| {
            status.progress = (total > 0).then(|| done as f32 / total as f32);
            status.detail = if total > 0 {
                format!("{} of {}", net::human_bytes(done), net::human_bytes(total))
            } else {
                net::human_bytes(done)
            };
        });
    })?;

    if let Some(url) = release.checksum {
        shared.update(|status| {
            status.progress = None;
            status.detail = "verifying".into();
        });
        match net::get_text(agent, &url) {
            Ok(text) => {
                let expected = text.split_whitespace().next().unwrap_or_default().to_lowercase();
                let actual = sha512(&tarball)?;
                if expected != actual {
                    std::fs::remove_file(&tarball).ok();
                    bail!("Proton download failed its checksum, try again");
                }
                shared.log("proton checksum ok");
            }
            // a missing checksum file is not worth blocking the install
            Err(err) => shared.log(format!("could not fetch checksum: {err}")),
        }
    }

    shared.update(|status| status.detail = "extracting".into());
    let dest = paths.proton();
    crate::paths::create_dir(&dest)?;
    let existing = dest.join(&release.tag);
    if existing.exists() {
        std::fs::remove_dir_all(&existing).ok();
    }
    unpack(&tarball, &dest)?;
    std::fs::remove_file(&tarball).ok();

    let dir = if existing.join("proton").is_file() {
        existing
    } else {
        detect_managed(paths)
            .map(|p| p.dir)
            .context("the Proton archive did not contain a proton script")?
    };

    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| release.tag.clone());
    shared.log(format!("installed {name}"));

    Ok(ProtonInstall {
        name,
        dir,
        source: ProtonSource::Managed,
    })
}

fn unpack(tarball: &Path, dest: &Path) -> Result<()> {
    let file = std::fs::File::open(tarball)
        .with_context(|| format!("cannot open {}", tarball.display()))?;
    let decoder = flate2::read::GzDecoder::new(std::io::BufReader::new(file));
    let mut archive = tar::Archive::new(decoder);
    archive.set_preserve_permissions(true);
    archive
        .unpack(dest)
        .with_context(|| format!("cannot unpack Proton into {} (disk full?)", dest.display()))
}

fn sha512(path: &Path) -> Result<String> {
    use std::io::Read;

    let file = std::fs::File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let mut reader = std::io::BufReader::new(file);
    let mut hasher = Sha512::new();
    let mut buffer = vec![0u8; 128 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .with_context(|| format!("cannot read {}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ASSETS: [&str; 4] = [
        "GE-Proton11-3-aarch64.sha512sum",
        "GE-Proton11-3-aarch64.tar.gz",
        "GE-Proton11-3.sha512sum",
        "GE-Proton11-3.tar.gz",
    ];

    #[test]
    fn skips_the_aarch64_tarball_on_x86_64() {
        assert_eq!(
            tarball_for_host(&ASSETS, "x86_64").as_deref(),
            Some("GE-Proton11-3.tar.gz")
        );
    }

    #[test]
    fn takes_the_aarch64_tarball_on_arm() {
        assert_eq!(
            tarball_for_host(&ASSETS, "aarch64").as_deref(),
            Some("GE-Proton11-3-aarch64.tar.gz")
        );
    }

    #[test]
    fn checksum_matches_the_tarball() {
        assert_eq!(checksum_for("GE-Proton11-3.tar.gz"), "GE-Proton11-3.sha512sum");
        assert_eq!(
            checksum_for("GE-Proton11-3-aarch64.tar.gz"),
            "GE-Proton11-3-aarch64.sha512sum"
        );
    }
}
