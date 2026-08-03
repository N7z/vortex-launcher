// SPDX-License-Identifier: AGPL-3.0-or-later
//! The stored session token. It is account access, so it lives in its own 0600 file.

use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub token: String,
    pub username: String,
    pub user_id: u64,
}

pub fn load(path: &Path) -> Option<Session> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn save(path: &Path, session: &Session) -> Result<()> {
    let parent = path.parent().context("session path has no parent")?;
    crate::paths::create_dir(parent)?;

    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(session).context("cannot serialize the session")?;
    {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)
            .with_context(|| format!("cannot write {}", tmp.display()))?;
        file.write_all(text.as_bytes())
            .with_context(|| format!("cannot write {}", tmp.display()))?;
    }
    // a file that already existed keeps its old mode, so set it either way
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600)).ok();
    std::fs::rename(&tmp, path).with_context(|| format!("cannot replace {}", path.display()))?;
    Ok(())
}

pub fn clear(path: &Path) {
    std::fs::remove_file(path).ok();
}

pub fn redact(line: &str) -> String {
    const KEYS: [&str; 3] = ["token=", "ticket=", "code="];
    const MASK: &str = "***";

    let mut out = line.to_owned();
    for key in KEYS {
        let mut cursor = 0;
        // to_ascii_lowercase keeps byte offsets, unlike to_lowercase
        while let Some(hit) = out.to_ascii_lowercase()[cursor..].find(key) {
            let start = cursor + hit + key.len();
            let end = out[start..]
                .find(|c: char| c == '&' || c == ';' || c == '"' || c.is_whitespace())
                .map(|offset| start + offset)
                .unwrap_or(out.len());
            if end > start {
                out.replace_range(start..end, MASK);
            }
            cursor = start + MASK.len().min(out.len() - start);
            if cursor >= out.len() {
                break;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hides_tokens() {
        assert_eq!(
            redact("running vortex://launch?token=abc123&game=3"),
            "running vortex://launch?token=***&game=3"
        );
        assert_eq!(redact("nothing secret here"), "nothing secret here");
    }

    #[test]
    fn hides_every_secret_in_one_line() {
        let line = "join ticket=xyz session_token=deadbeef done";
        let out = redact(line);
        assert!(!out.contains("xyz"), "{out}");
        assert!(!out.contains("deadbeef"), "{out}");
    }
}
