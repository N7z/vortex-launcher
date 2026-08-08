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
const RELEASE_LIST: &str = "https://api.github.com/repos/GloriousEggroll/proton-ge-custom/releases?per_page=40";
const RELEASE_BY_TAG: &str = "https://api.github.com/repos/GloriousEggroll/proton-ge-custom/releases/tags/";

/// where steam keeps proton builds
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

/// a build on disk, plus whether this host can actually load it
#[derive(Clone, Debug)]
pub struct Build {
    pub name: String,
    pub dir: PathBuf,
    pub source: ProtonSource,
    /// highest GLIBC_x.y the build asks for, None when it cannot be read
    pub needs_glibc: Option<(u32, u32)>,
}

impl Build {
    /// unknown requirements count as fine, a guess is no reason to hide a build
    pub fn runs_here(&self) -> bool {
        match (self.needs_glibc, host_glibc()) {
            (Some(needs), Some(host)) => needs <= host,
            _ => true,
        }
    }

    pub fn install(&self) -> ProtonInstall {
        ProtonInstall {
            name: self.name.clone(),
            dir: self.dir.clone(),
            source: self.source,
        }
    }
}

/// glibc of the running system, read once
pub fn host_glibc() -> Option<(u32, u32)> {
    use std::sync::OnceLock;

    static CACHE: OnceLock<Option<(u32, u32)>> = OnceLock::new();
    *CACHE.get_or_init(|| {
        let getconf = std::process::Command::new("getconf")
            .arg("GNU_LIBC_VERSION")
            .output()
            .ok()
            .filter(|out| out.status.success())
            .and_then(|out| parse_glibc(&String::from_utf8_lossy(&out.stdout)));
        getconf.or_else(|| {
            let out = std::process::Command::new("ldd").arg("--version").output().ok()?;
            let text = String::from_utf8_lossy(&out.stdout);
            parse_glibc(text.lines().next()?)
        })
    })
}

/// the last x.y in the line, which is where both getconf and ldd put it
fn parse_glibc(text: &str) -> Option<(u32, u32)> {
    text.split_whitespace().rev().find_map(|word| {
        let word = word.trim_matches(|c: char| !c.is_ascii_digit() && c != '.');
        let (major, minor) = word.split_once('.')?;
        Some((major.parse().ok()?, minor.parse().ok()?))
    })
}

/// wine's unix side is what breaks first on an old host, so ask that file
fn glibc_needed_by(dir: &Path) -> Option<(u32, u32)> {
    [
        "files/lib/wine/x86_64-unix/ntdll.so",
        "files/lib64/wine/x86_64-unix/ntdll.so",
        "files/bin/wineserver",
    ]
    .iter()
    .map(|rel| dir.join(rel))
    .find(|path| path.is_file())
    .and_then(|path| max_glibc_tag(&path))
}

/// the version tags sit in .dynstr as plain ascii, so scanning beats parsing the ELF
fn max_glibc_tag(path: &Path) -> Option<(u32, u32)> {
    const NEEDLE: &[u8] = b"GLIBC_";

    let bytes = std::fs::read(path).ok()?;
    bytes
        .windows(NEEDLE.len())
        .enumerate()
        .filter(|(_, window)| *window == NEEDLE)
        .filter_map(|(at, _)| {
            let tail = &bytes[at + NEEDLE.len()..];
            let end = tail
                .iter()
                .position(|b| !b.is_ascii_digit() && *b != b'.')
                .unwrap_or(tail.len());
            parse_glibc(std::str::from_utf8(&tail[..end]).ok()?)
        })
        .max()
}

/// every build the launcher could use, managed first, newest-looking last
pub fn available(paths: &Paths) -> Vec<Build> {
    let managed = builds_in(&paths.proton())
        .into_iter()
        .map(|(name, dir)| (name, dir, ProtonSource::Managed));
    let system: Vec<_> = system_candidates().iter().flat_map(|dir| builds_in(dir)).collect();
    let system = system
        .into_iter()
        .map(|(name, dir)| (name, dir, ProtonSource::System));

    let mut seen = std::collections::HashSet::new();
    let mut builds: Vec<Build> = managed
        .chain(system)
        .filter(|(_, dir, _)| seen.insert(dir.clone()))
        .map(|(name, dir, source)| Build {
            needs_glibc: glibc_needed_by(&dir),
            name,
            dir,
            source,
        })
        .collect();
    // usable ones first, then managed, then by name
    builds.sort_by(|a, b| {
        b.runs_here()
            .cmp(&a.runs_here())
            .then_with(|| a.source.cmp(&b.source))
            .then_with(|| a.name.cmp(&b.name))
    });
    builds
}

/// what to use when nothing is configured yet. a build this host cannot load is
/// only picked when there is nothing else, so the error at least names a proton
pub fn detect(paths: &Paths) -> Option<ProtonInstall> {
    let builds = available(paths);
    let pick = |managed: bool, runs: bool| {
        builds
            .iter()
            .filter(|b| (b.source == ProtonSource::Managed) == managed && b.runs_here() == runs)
            // highest name last, which is the newest in practice
            .max_by(|a, b| a.name.cmp(&b.name))
    };
    pick(true, true)
        .or_else(|| pick(false, true))
        .or_else(|| pick(true, false))
        .or_else(|| pick(false, false))
        .map(Build::install)
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

/// tags offered in the picker, newest first. an empty list just means no network
pub fn releases(agent: &ureq::Agent) -> Result<Vec<String>> {
    let body = net::get_text(agent, RELEASE_LIST).context("cannot ask GitHub for GE-Proton")?;
    let json: serde_json::Value =
        serde_json::from_str(&body).context("GitHub sent something that is not JSON")?;
    Ok(json
        .as_array()
        .context("GitHub sent no release list")?
        .iter()
        .filter_map(|release| release["tag_name"].as_str())
        .map(str::to_owned)
        .collect())
}

fn latest_release(agent: &ureq::Agent) -> Result<Release> {
    let body = net::get_text(agent, LATEST_RELEASE).context("cannot ask GitHub for GE-Proton")?;
    release_from(&serde_json::from_str(&body).context("GitHub sent something that is not JSON")?)
}

fn release_by_tag(agent: &ureq::Agent, tag: &str) -> Result<Release> {
    let url = format!("{RELEASE_BY_TAG}{tag}");
    let body = net::get_text(agent, &url).with_context(|| format!("cannot find {tag} on GitHub"))?;
    release_from(&serde_json::from_str(&body).context("GitHub sent something that is not JSON")?)
}

fn release_from(json: &serde_json::Value) -> Result<Release> {
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

/// `tag` picks a specific build, None takes the newest one
pub fn install(
    agent: &ureq::Agent,
    paths: &Paths,
    shared: &Arc<Shared>,
    cancel: &AtomicBool,
    tag: Option<&str>,
) -> Result<ProtonInstall> {
    let release = match tag {
        Some(tag) => release_by_tag(agent, tag)?,
        None => latest_release(agent)?,
    };
    shared.log(format!("installing proton {}", release.tag));

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
        // the tag and the directory inside the tarball do not always match
        builds_in(&paths.proton())
            .into_iter()
            .max_by(|a, b| a.0.cmp(&b.0))
            .map(|(_, dir)| dir)
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

#[cfg(test)]
mod glibc_tests {
    use super::*;

    #[test]
    fn reads_the_version_from_getconf_and_ldd_shapes() {
        assert_eq!(parse_glibc("glibc 2.35"), Some((2, 35)));
        assert_eq!(parse_glibc("ldd (Ubuntu GLIBC 2.35-0ubuntu3.8) 2.35"), Some((2, 35)));
        assert_eq!(parse_glibc("nothing here"), None);
    }

    #[test]
    fn orders_minor_versions_as_numbers() {
        assert!(parse_glibc("glibc 2.9") < parse_glibc("glibc 2.35"));
        assert!(parse_glibc("glibc 2.38") < parse_glibc("glibc 3.0"));
    }

    #[test]
    fn takes_the_highest_tag_in_the_file() {
        let dir = std::env::temp_dir().join("vortex-glibc-scan");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("fake.so");
        std::fs::write(&file, b"\x00GLIBC_2.2.5\x00GLIBC_2.38\x00GLIBC_2.9\x00").unwrap();
        assert_eq!(max_glibc_tag(&file), Some((2, 38)));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_build_with_unreadable_requirements_is_not_hidden() {
        let build = Build {
            name: "GE-Proton9-27".into(),
            dir: PathBuf::from("/nowhere"),
            source: ProtonSource::Managed,
            needs_glibc: None,
        };
        assert!(build.runs_here());
    }
}
