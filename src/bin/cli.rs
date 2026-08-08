// SPDX-License-Identifier: AGPL-3.0-or-later
//! Headless companion to the GUI launcher. `vortex-launcher-cli login` signs in from the
//! terminal; with no arguments it starts Vortex.exe itself (no game join URI).

use std::io::{BufRead, Write};
use std::process::Command;

use anyhow::{bail, Context, Result};
use vortex_launcher::{
    auth, config::Config, ipc, launch, net, paths::Paths, session, state::Shared, studio,
};

const HELP: &str = "\
vortex-launcher-cli — headless Vortex launcher

USAGE:
    vortex-launcher-cli              start Vortex.exe (no game is joined)
    vortex-launcher-cli login        sign in to playvortex.io and store the session
    vortex-launcher-cli logout       forget the stored session
    vortex-launcher-cli whoami       show the signed-in account
    vortex-launcher-cli games        list games and player counts
    vortex-launcher-cli play <id>    launch straight into a game
    vortex-launcher-cli studio       install (first time) and open Vortex Studio
    vortex-launcher-cli vortex://…   open a link from the browser (no GUI)
    vortex-launcher-cli help         show this help

Installing Vortex and Proton still happens in the GUI (vortex-launcher).";

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let paths = Paths::discover()?;
    let args: Vec<String> = std::env::args().skip(1).collect();

    match args.first().map(String::as_str) {
        None => launch_game(&paths, None),
        Some("login") => login(&paths),
        Some("logout") => {
            session::clear(&paths.session_file());
            println!("signed out");
            Ok(())
        }
        Some("whoami") => whoami(&paths),
        Some("games") => games(),
        Some("studio") => open_studio(&paths),
        Some("play") => {
            let id: u64 = args
                .get(1)
                .context("usage: vortex-launcher-cli play <game-id>")?
                .parse()
                .context("the game id must be a number (see `games`)")?;
            play(&paths, id)
        }
        Some("help") | Some("--help") | Some("-h") => {
            println!("{HELP}");
            Ok(())
        }
        Some(uri) if auth::is_launch_uri(uri) => {
            if ipc::send(uri) {
                return Ok(());
            }
            launch_program(&paths, launch::Target::Game, Some(uri))
        }
        Some(uri) if auth::is_studio_uri(uri) => {
            if ipc::send(uri) {
                return Ok(());
            }
            launch_program(&paths, launch::Target::Studio, Some(uri))
        }
        Some(other) => bail!("unknown command `{other}`\n\n{HELP}"),
    }
}

fn login(paths: &Paths) -> Result<()> {
    let username = prompt("Username: ")?;
    let password = prompt_hidden("Password: ")?;

    let mut result = auth::login(&username, &password)?;
    if let auth::Login::Needs2fa(pending) = &result {
        let code = prompt("2FA code: ")?;
        result = auth::login_2fa(pending, &code)?;
    }
    let token = match result {
        auth::Login::Done(token) => token,
        auth::Login::Needs2fa(_) => bail!("the server asked for 2FA twice, try again"),
    };

    let account = auth::account(&token)?;
    session::save(
        &paths.session_file(),
        &session::Session {
            token,
            username: account.username.clone(),
            user_id: account.id,
        },
    )?;
    println!("signed in as {}", account.username);
    Ok(())
}

fn whoami(paths: &Paths) -> Result<()> {
    let session = session::load(&paths.session_file()).context("not signed in, run `login`")?;
    match auth::account(&session.token) {
        Ok(account) => println!("{} (id {})", account.username, account.id),
        Err(err) => bail!("stored session for {} no longer works: {err:#}", session.username),
    }
    Ok(())
}

fn games() -> Result<()> {
    let games = auth::games()?;
    if games.is_empty() {
        println!("no games listed right now");
        return Ok(());
    }
    for game in games {
        println!("{:>6}  {:<30} {} playing", game.id, game.name, game.players);
    }
    Ok(())
}

fn play(paths: &Paths, game_id: u64) -> Result<()> {
    let session = session::load(&paths.session_file()).context("not signed in, run `login`")?;
    let uri = auth::play_uri(&session.token, game_id, None)?;
    launch_game(paths, Some(&uri))
}

fn open_studio(paths: &Paths) -> Result<()> {
    let session = session::load(&paths.session_file()).context("not signed in, run `login`")?;
    let mut config = Config::load(&paths.config_file());
    if config.proton.is_none() {
        bail!("Proton is not installed yet, run the GUI (vortex-launcher) once to install");
    }

    let shared = Shared::new(&paths.logs().join("cli.log"));
    if !config.studio_ready() {
        paths.ensure()?;
        println!("downloading Vortex Studio...");
        let agent = net::agent();
        let cancel = std::sync::atomic::AtomicBool::new(false);
        config.studio = Some(studio::install(
            &agent,
            paths,
            &shared,
            &cancel,
            &session.token,
        )?);
        config.save(&paths.config_file())?;
        println!("studio installed");
    }

    let uri = auth::studio_uri(&session.token)?;
    launch_with(paths, &config, &shared, launch::Target::Studio, Some(&uri))
}

fn launch_game(paths: &Paths, uri: Option<&str>) -> Result<()> {
    launch_program(paths, launch::Target::Game, uri)
}

fn launch_program(paths: &Paths, target: launch::Target, uri: Option<&str>) -> Result<()> {
    let config = Config::load(&paths.config_file());
    if config.proton.is_none() {
        bail!("Proton is not installed yet, run the GUI (vortex-launcher) once to install");
    }
    match target {
        launch::Target::Game if config.game.is_none() => {
            bail!("Vortex is not installed yet, run the GUI (vortex-launcher) once to install")
        }
        launch::Target::Studio if config.studio.is_none() => {
            bail!("Vortex Studio is not installed yet, run `vortex-launcher-cli studio` first")
        }
        _ => {}
    }

    let shared = Shared::new(&paths.logs().join("cli.log"));
    launch_with(paths, &config, &shared, target, uri)
}

fn launch_with(
    paths: &Paths,
    config: &Config,
    shared: &std::sync::Arc<Shared>,
    target: launch::Target,
    uri: Option<&str>,
) -> Result<()> {
    let (what, log) = match target {
        launch::Target::Game => ("vortex", "game.log"),
        launch::Target::Studio => ("vortex studio", "studio.log"),
    };
    launch::launch_target(paths, config, shared, target, uri)?;
    println!(
        "{what} is running (output in {})",
        paths.logs().join(log).display()
    );

    // launch() hands the child to a background thread; stay alive until it exits
    // so the terminal user sees the outcome and a fast crash is not silent
    loop {
        std::thread::sleep(std::time::Duration::from_millis(300));
        let (running, error) = shared
            .read(|status| {
                let running = match target {
                    launch::Target::Game => status.game_running,
                    launch::Target::Studio => status.studio_running,
                };
                (running, status.error.clone())
            })
            .unwrap_or((false, None));
        if let Some(error) = error {
            bail!("{error}");
        }
        if !running {
            println!("{what} exited");
            return Ok(());
        }
    }
}

fn prompt(label: &str) -> Result<String> {
    print!("{label}");
    std::io::stdout().flush().ok();
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .context("cannot read from the terminal")?;
    let value = line.trim().to_owned();
    if value.is_empty() {
        bail!("nothing entered");
    }
    Ok(value)
}

/// no extra deps: flip echo off with stty for the password, back on after
fn prompt_hidden(label: &str) -> Result<String> {
    print!("{label}");
    std::io::stdout().flush().ok();

    let echo_off = Command::new("stty").arg("-echo").status().is_ok_and(|s| s.success());
    let mut line = String::new();
    let read = std::io::stdin().lock().read_line(&mut line);
    if echo_off {
        Command::new("stty").arg("echo").status().ok();
        println!();
    }
    read.context("cannot read from the terminal")?;

    let value = line.trim_end_matches(['\r', '\n']).to_owned();
    if value.is_empty() {
        bail!("nothing entered");
    }
    Ok(value)
}
