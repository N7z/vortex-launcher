// SPDX-License-Identifier: AGPL-3.0-or-later
//! Starting Vortex.exe under Proton, without Steam.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;

use anyhow::{bail, Context, Result};

use crate::config::Config;
use crate::paths::Paths;
use crate::state::Shared;

/// the proton script is python, so we need an interpreter on the host
fn python() -> Result<PathBuf> {
    let path = std::env::var_os("PATH").context("PATH is not set")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join("python3");
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    bail!("python3 was not found.")
}

pub fn launch(
    paths: &Paths,
    config: &Config,
    shared: &Arc<Shared>,
    uri: Option<&str>,
) -> Result<()> {
    let game = config
        .game
        .as_ref()
        .context("Vortex is not installed yet")?;
    let proton = config
        .proton
        .as_ref()
        .context("Proton is not installed yet")?;

    if !game.exe.is_file() {
        bail!("{} is missing, reinstall Vortex", game.exe.display());
    }
    let script = proton.dir.join("proton");
    if !script.is_file() {
        bail!("{} is missing, reinstall Proton", script.display());
    }
    let python = python()?;
    let working_dir = game.exe.parent().unwrap_or(&paths.game()).to_path_buf();

    paths.ensure()?;
    let log_path = paths.logs().join("game.log");
    let log = std::fs::File::create(&log_path)
        .with_context(|| format!("cannot write {}", log_path.display()))?;

    let mut command = Command::new(python);
    command
        .arg(&script)
        .arg("run")
        .arg(&game.exe)
        .current_dir(&working_dir);
    if let Some(uri) = uri {
        if !crate::auth::is_launch_uri(uri) {
            bail!("that is not a vortex:// launch link");
        }
        command.arg(uri);
    }
    command
        .env("STEAM_COMPAT_DATA_PATH", paths.prefix())
        .env("STEAM_COMPAT_CLIENT_INSTALL_PATH", paths.compat_client())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if !config.allow_self_update {
        command.env("VORTEX_NO_UPDATE", "1");
    }
    if config.native_shader_compiler {
        // proton rebuilds WINEDLLOVERRIDES itself and merges PROTON_DLL_OVERRIDES
        // into it, so set both and keep whatever the user already exported
        command.env(
            "PROTON_DLL_OVERRIDES",
            merge_overrides("PROTON_DLL_OVERRIDES"),
        );
        command.env("WINEDLLOVERRIDES", merge_overrides("WINEDLLOVERRIDES"));
    }

    let mut child = command
        .spawn()
        .with_context(|| format!("cannot start Proton ({})", script.display()))?;

    shared.log(format!("running vortex via {}", proton.name));
    shared.update(|status| status.game_running = true);

    let log = Arc::new(std::sync::Mutex::new(log));
    let mut pipes = Vec::new();
    if let Some(out) = child.stdout.take() {
        pipes.push(pipe(out, Arc::clone(&log)));
    }
    if let Some(err) = child.stderr.take() {
        pipes.push(pipe(err, Arc::clone(&log)));
    }

    let shared = Arc::clone(shared);
    let started = std::time::Instant::now();
    std::thread::spawn(move || {
        let result = child.wait();
        for pipe in pipes {
            pipe.join().ok();
        }
        match result {
            Ok(status) if status.success() => shared.log("vortex exited"),
            Ok(status) => {
                shared.log(format!("vortex exited with {status}"));
                // dying this fast means it never opened, so say so instead of just logging
                if started.elapsed() < std::time::Duration::from_secs(20) {
                    let code = status
                        .code()
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "a signal".into());
                    let message = format!(
                        "Vortex closed immediately (exit {code}). Full output in {}",
                        log_path.display()
                    );
                    shared.update(|status| status.error = Some(message));
                }
            }
            Err(err) => shared.log(format!("lost track of vortex: {err}")),
        }
        shared.update(|status| status.game_running = false);
    });

    Ok(())
}

/// our override appended to whatever `name` already holds, ours last so it wins
fn merge_overrides(name: &str) -> String {
    match std::env::var(name) {
        Ok(existing) if !existing.trim().is_empty() => {
            format!("{existing};{}", crate::shaders::OVERRIDE)
        }
        _ => crate::shaders::OVERRIDE.to_owned(),
    }
}

fn pipe<R: std::io::Read + Send + 'static>(
    reader: R,
    log: Arc<std::sync::Mutex<std::fs::File>>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        for line in BufReader::new(reader).lines().map_while(Result::ok) {
            // the launch uri carries a credential and the client echoes its argv
            let line = crate::session::redact(&line);
            if let Ok(mut file) = log.lock() {
                writeln!(file, "{line}").ok();
            }
        }
    })
}
