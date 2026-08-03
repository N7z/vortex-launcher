// SPDX-License-Identifier: AGPL-3.0-or-later
//! Signing in to playvortex.io and minting a vortex:// launch URI.

use anyhow::{bail, Context, Result};

pub const BASE: &str = "https://playvortex.io";

pub enum Login {
    Done(String),
    /// server wants the 2fa code, carries the token that ties the two requests
    Needs2fa(String),
}

#[derive(Clone, Debug)]
pub struct Account {
    pub id: u64,
    pub username: String,
}

/// no redirects, the session cookie rides on the 302 itself. status as error off,
/// or a 401 becomes "http status: 401" instead of what the server actually said
fn agent() -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .user_agent(crate::net::USER_AGENT)
        .max_redirects(0)
        .http_status_as_error(false)
        .timeout_global(Some(std::time::Duration::from_secs(30)))
        .build();
    ureq::Agent::new_with_config(config)
}

pub fn login(username: &str, password: &str) -> Result<Login> {
    post_login(
        &format!("{BASE}/login"),
        &[
            ("username", username),
            ("password", password),
            ("fingerprint", ""),
            ("fp_token", ""),
        ],
    )
}

pub fn login_2fa(pending_token: &str, code: &str) -> Result<Login> {
    post_login(
        &format!("{BASE}/login/2fa"),
        &[
            ("code", code),
            ("pending_token", pending_token),
            ("fingerprint", ""),
            ("fp_token", ""),
        ],
    )
}

fn post_login(url: &str, form: &[(&str, &str)]) -> Result<Login> {
    let mut response = agent()
        .post(url)
        .send_form(form.iter().copied())
        .with_context(|| format!("cannot reach {url}"))?;

    let status = response.status().as_u16();
    let cookie = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find_map(session_token_from_cookie);
    let body = response.body_mut().read_to_string().unwrap_or_default();

    if (200..400).contains(&status) {
        if let Some(pending) = pending_token(&body) {
            return Ok(Login::Needs2fa(pending));
        }
        if let Some(token) = cookie {
            return Ok(Login::Done(token));
        }
        bail!("the server accepted the login but sent no session cookie");
    }

    bail!("{}", detail(&body).unwrap_or_else(|| match status {
        401 | 403 => "wrong username or password".into(),
        429 => "too many attempts, wait a moment".into(),
        _ => format!("the server answered HTTP {status}"),
    }))
}

/// `session_token=abc; Path=/; HttpOnly` -> `abc`
fn session_token_from_cookie(header: &str) -> Option<String> {
    let pair = header.split(';').next()?.trim();
    let (name, value) = pair.split_once('=')?;
    let value = value.trim().trim_matches('"');
    (name.trim() == "session_token" && !value.is_empty()).then(|| value.to_owned())
}

fn pending_token(body: &str) -> Option<String> {
    let json: serde_json::Value = serde_json::from_str(body).ok()?;
    json.get("requires_2fa")?.as_bool()?.then(|| {
        json.get("pending_token")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_owned()
    })
}

fn detail(body: &str) -> Option<String> {
    let json: serde_json::Value = serde_json::from_str(body).ok()?;
    json.get("detail")?.as_str().map(str::to_owned)
}

pub fn account(token: &str) -> Result<Account> {
    let mut response = agent()
        .get(format!("{BASE}/api/users/me"))
        .header("Cookie", format!("session_token={token}"))
        .call()
        .context("cannot reach playvortex.io")?;

    let status = response.status().as_u16();
    let body = response
        .body_mut()
        .read_to_string()
        .context("cannot read the account response")?;

    if status == 401 || status == 403 {
        bail!(
            "session expired ({})",
            detail(&body).unwrap_or_else(|| format!("HTTP {status}"))
        );
    }
    if !(200..300).contains(&status) {
        bail!("the account check answered HTTP {status}");
    }

    let json: serde_json::Value =
        serde_json::from_str(&body).context("the account response was not JSON")?;
    Ok(Account {
        id: json["id"].as_u64().unwrap_or_default(),
        username: json["username"].as_str().unwrap_or("unknown").to_owned(),
    })
}

#[derive(Clone, Debug)]
pub struct Game {
    pub id: u64,
    pub name: String,
    pub players: u64,
}

pub fn games() -> Result<Vec<Game>> {
    let mut response = agent()
        .get(format!("{BASE}/api/games"))
        .call()
        .context("cannot fetch the game list")?;
    let body = response
        .body_mut()
        .read_to_string()
        .context("cannot read the game list")?;
    let json: serde_json::Value =
        serde_json::from_str(&body).context("the game list was not JSON")?;
    let items = json.as_array().context("the game list was not a list")?;

    Ok(items
        .iter()
        .filter_map(|game| {
            Some(Game {
                id: game["id"].as_u64()?,
                name: game["name"].as_str()?.to_owned(),
                players: game["player_count"].as_u64().unwrap_or_default(),
            })
        })
        .collect())
}

/// the play page carries the launch URI inline, which is the whole handoff
pub fn play_uri(token: &str, game_id: u64, instance: Option<&str>) -> Result<String> {
    let url = match instance {
        Some(id) => format!("{BASE}/games/{game_id}/play?instance={id}"),
        None => format!("{BASE}/games/{game_id}/play"),
    };
    let mut response = agent()
        .get(&url)
        .header("Cookie", format!("session_token={token}"))
        .call()
        .context("cannot reach the play page")?;

    let status = response.status().as_u16();
    if status == 401 || status == 403 || (300..400).contains(&status) {
        bail!("session expired (the play page answered HTTP {status})");
    }
    if !(200..300).contains(&status) {
        bail!("the play page answered HTTP {status}");
    }

    let html = response
        .body_mut()
        .read_to_string()
        .context("cannot read the play page")?;
    extract_uri(&html).context("the play page had no vortex:// link, the site may have changed")
}

fn extract_uri(html: &str) -> Option<String> {
    let start = html.find("vortex://")?;
    let rest = &html[start..];
    let end = rest
        .find(|c: char| c == '"' || c == '\'' || c == '<' || c.is_whitespace())
        .unwrap_or(rest.len());
    Some(rest[..end].to_owned())
}

/// nothing but a vortex:// uri ever reaches the game's argv
pub fn is_launch_uri(uri: &str) -> bool {
    uri.starts_with("vortex://")
        && uri.len() <= 4096
        && !uri.contains(char::is_whitespace)
        && !uri.contains('\0')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_session_cookie() {
        assert_eq!(
            session_token_from_cookie("session_token=abc123; Path=/; HttpOnly; Secure").as_deref(),
            Some("abc123")
        );
        assert_eq!(session_token_from_cookie("other=x; Path=/"), None);
        assert_eq!(session_token_from_cookie("session_token=; Path=/"), None);
    }

    #[test]
    fn spots_the_2fa_handoff() {
        let body = r#"{"requires_2fa":true,"pending_token":"pt-9"}"#;
        assert_eq!(pending_token(body).as_deref(), Some("pt-9"));
        assert_eq!(pending_token(r#"{"ok":true}"#), None);
    }

    #[test]
    fn reads_the_server_error() {
        assert_eq!(
            detail(r#"{"detail":"Invalid credentials"}"#).as_deref(),
            Some("Invalid credentials")
        );
    }

    #[test]
    fn pulls_the_uri_out_of_the_page() {
        let html = r#"<a id="go" href="vortex://launch?token=abc&game=3">Play</a>"#;
        assert_eq!(
            extract_uri(html).as_deref(),
            Some("vortex://launch?token=abc&game=3")
        );
        assert_eq!(extract_uri("<p>nothing here</p>"), None);
    }

    #[test]
    fn rejects_anything_but_a_launch_uri() {
        assert!(is_launch_uri("vortex://launch?token=abc"));
        assert!(!is_launch_uri("https://evil.example/x"));
        assert!(!is_launch_uri("vortex://a b"));
    }
}
