// SPDX-License-Identifier: AGPL-3.0-or-later
//! HTTP: resumable downloads and cheap remote-version probes.

use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};

pub const USER_AGENT: &str = concat!("vortex-launcher/", env!("CARGO_PKG_VERSION"));

/// user hit stop, kept distinct so the UI does not show it as a failure
#[derive(Debug)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("cancelled")
    }
}

impl std::error::Error for Cancelled {}

pub fn is_cancelled(err: &anyhow::Error) -> bool {
    err.chain().any(|e| e.is::<Cancelled>())
}

pub fn agent() -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .user_agent(USER_AGENT)
        // connect only, a 400 MB download must not race a global timer
        .timeout_connect(Some(Duration::from_secs(20)))
        // every caller checks the status itself and says something useful about it
        .http_status_as_error(false)
        .build();
    ureq::Agent::new_with_config(config)
}

#[derive(Clone, Debug, Default)]
pub struct Remote {
    pub size: u64,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

impl Remote {
    pub fn differs_from(&self, etag: Option<&str>, last_modified: Option<&str>, size: u64) -> bool {
        if let (Some(remote), Some(local)) = (&self.etag, etag) {
            return remote != local;
        }
        if let (Some(remote), Some(local)) = (&self.last_modified, last_modified) {
            return remote != local;
        }
        self.size != size
    }
}

/// one-byte ranged GET, because HEAD on the download route answers 404
pub fn probe(agent: &ureq::Agent, url: &str) -> Result<Remote> {
    let response = agent
        .get(url)
        .header("Range", "bytes=0-0")
        .call()
        .with_context(|| format!("cannot reach {url}"))?;

    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        bail!("{url} answered HTTP {status}");
    }

    let header = |name: &str| {
        response
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    };

    let size = header("content-range")
        .as_deref()
        .and_then(parse_total_from_content_range)
        .or_else(|| header("content-length").and_then(|v| v.parse().ok()))
        .unwrap_or(0);

    Ok(Remote {
        size,
        etag: header("etag"),
        last_modified: header("last-modified"),
    })
}

/// bytes 0-0/32733650 -> 32733650
fn parse_total_from_content_range(value: &str) -> Option<u64> {
    value.rsplit('/').next()?.trim().parse().ok()
}

pub fn download(
    agent: &ureq::Agent,
    url: &str,
    dest: &Path,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(u64, u64),
) -> Result<()> {
    let already = std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0);

    let mut request = agent.get(url);
    if already > 0 {
        request = request.header("Range", format!("bytes={already}-"));
    }
    let response = request.call().with_context(|| format!("cannot download {url}"))?;

    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        bail!("{url} answered HTTP {status}");
    }

    // 206 resumed us, a plain 200 means the range was ignored so the partial is dead
    let resumed = status == 206 && already > 0;
    let mut done = if resumed { already } else { 0 };

    let header = |name: &str| response.headers().get(name).and_then(|v| v.to_str().ok());
    let total = header("content-range")
        .and_then(parse_total_from_content_range)
        .or_else(|| header("content-length").and_then(|v| v.parse::<u64>().ok()).map(|len| done + len))
        .unwrap_or(0);

    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(!resumed)
        .open(dest)
        .with_context(|| format!("cannot open {}", dest.display()))?;
    if resumed {
        file.seek(SeekFrom::End(0))
            .with_context(|| format!("cannot seek {}", dest.display()))?;
    }

    let mut reader = response.into_body().into_reader();
    let mut buffer = vec![0u8; 128 * 1024];
    on_progress(done, total);

    loop {
        if cancel.load(Ordering::Relaxed) {
            file.flush().ok();
            return Err(anyhow!(Cancelled));
        }
        let read = reader
            .read(&mut buffer)
            .with_context(|| format!("connection lost while downloading {url}"))?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])
            .with_context(|| format!("cannot write to {} (disk full?)", dest.display()))?;
        done += read as u64;
        on_progress(done, total);
    }

    file.flush().with_context(|| format!("cannot flush {}", dest.display()))?;

    if total > 0 && done != total {
        bail!("download ended early: got {} of {}", human_bytes(done), human_bytes(total));
    }
    Ok(())
}

pub fn get_text(agent: &ureq::Agent, url: &str) -> Result<String> {
    let mut response = agent
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .call()
        .with_context(|| format!("cannot reach {url}"))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        bail!("{url} answered HTTP {status}");
    }
    response
        .body_mut()
        .read_to_string()
        .with_context(|| format!("cannot read response from {url}"))
}

pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_content_range_total() {
        assert_eq!(parse_total_from_content_range("bytes 0-0/32733650"), Some(32_733_650));
        assert_eq!(parse_total_from_content_range("bytes 0-0/*"), None);
    }

    #[test]
    fn formats_bytes() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(32_733_650), "31.2 MiB");
    }

    #[test]
    fn etag_wins_over_size() {
        let remote = Remote {
            size: 10,
            etag: Some("\"a\"".into()),
            last_modified: None,
        };
        assert!(!remote.differs_from(Some("\"a\""), None, 999));
        assert!(remote.differs_from(Some("\"b\""), None, 10));
    }
}
