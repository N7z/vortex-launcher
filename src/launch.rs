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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Game,
    Studio,
}

impl Target {
    fn name(self) -> &'static str {
        match self {
            Target::Game => "Vortex",
            Target::Studio => "Vortex Studio",
        }
    }

    fn log_file(self) -> &'static str {
        match self {
            Target::Game => "game.log",
            Target::Studio => "studio.log",
        }
    }

    fn accepts(self, uri: &str) -> bool {
        match self {
            Target::Game => crate::auth::is_launch_uri(uri),
            Target::Studio => crate::auth::is_studio_uri(uri),
        }
    }
}

pub fn launch(
    paths: &Paths,
    config: &Config,
    shared: &Arc<Shared>,
    uri: Option<&str>,
) -> Result<()> {
    launch_target(paths, config, shared, Target::Game, uri)
}

pub fn launch_target(
    paths: &Paths,
    config: &Config,
    shared: &Arc<Shared>,
    target: Target,
    uri: Option<&str>,
) -> Result<()> {
    let game = match target {
        Target::Game => config.game.as_ref().context("Vortex is not installed yet")?,
        Target::Studio => config
            .studio
            .as_ref()
            .context("Vortex Studio is not installed yet")?,
    };
    let proton = config
        .proton
        .as_ref()
        .context("Proton is not installed yet")?;

    if !game.exe.is_file() {
        bail!(
            "{} is missing, reinstall {}",
            game.exe.display(),
            target.name()
        );
    }
    let script = proton.dir.join("proton");
    if !script.is_file() {
        bail!("{} is missing, reinstall Proton", script.display());
    }
    let python = python()?;
    let working_dir = game.exe.parent().unwrap_or(&paths.game()).to_path_buf();

    paths.ensure()?;
    let log_path = paths.logs().join(target.log_file());
    let log = std::fs::File::create(&log_path)
        .with_context(|| format!("cannot write {}", log_path.display()))?;

    let mut command = Command::new(python);
    command
        .arg(&script)
        .arg("run")
        .arg(&game.exe)
        .current_dir(&working_dir);
    if let Some(uri) = uri {
        if !target.accepts(uri) {
            bail!("that is not a launch link for {}", target.name());
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

    let name = target.name();
    shared.log(format!("running {name} via {}", proton.name));
    shared.update(|status| match target {
        Target::Game => status.game_running = true,
        Target::Studio => status.studio_running = true,
    });

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
            Ok(status) if status.success() => shared.log(format!("{name} exited")),
            Ok(status) => {
                shared.log(format!("{name} exited with {status}"));
                // dying this fast means it never opened, so say so instead of just logging
                if started.elapsed() < std::time::Duration::from_secs(20) {
                    let code = status
                        .code()
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "a signal".into());
                    let message = format!(
                        "{name} closed immediately (exit {code}). Full output in {}",
                        log_path.display()
                    );
                    shared.update(|status| status.error = Some(message));
                }
            }
            Err(err) => shared.log(format!("lost track of {name}: {err}")),
        }
        shared.update(|status| match target {
            Target::Game => status.game_running = false,
            Target::Studio => status.studio_running = false,
        });
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
