// SPDX-License-Identifier: AGPL-3.0-or-later
//! Headless companion to the GUI launcher. `vortex-launcher-cli login` signs in from the
//! terminal; with no arguments it starts Vortex.exe itself (no game join URI).

use std::io::{BufRead, Write};
use std::process::Command;

use anyhow::{bail, Context, Result};
use vortex_launcher::{auth, config::Config, ipc, launch, paths::Paths, session, state::Shared};

const HELP: &str = "\
vortex-launcher-cli — headless Vortex launcher

USAGE:
    vortex-launcher-cli              start Vortex.exe (no game is joined)
    vortex-launcher-cli login        sign in to playvortex.io and store the session
    vortex-launcher-cli logout       forget the stored session
    vortex-launcher-cli whoami       show the signed-in account
    vortex-launcher-cli games        list games and player counts
    vortex-launcher-cli play <id>    launch straight into a game
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
        // the browser hands us a vortex:// link; a launcher window that is already
        // open should keep it, otherwise we launch it here with no GUI at all
        Some(uri) if auth::is_launch_uri(uri) => {
            if ipc::send(uri) {
                return Ok(());
            }
            launch_game(&paths, Some(uri))
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

fn launch_game(paths: &Paths, uri: Option<&str>) -> Result<()> {
    let config = Config::load(&paths.config_file());
    if config.game.is_none() || config.proton.is_none() {
        bail!("Vortex or Proton is not installed yet, run the GUI (vortex-launcher) once to install");
    }

    let shared = Shared::new(&paths.logs().join("cli.log"));
    launch::launch(paths, &config, &shared, uri)?;
    println!(
        "vortex is running (output in {})",
        paths.logs().join("game.log").display()
    );

    // launch() hands the child to a background thread; stay alive until it exits
    // so the terminal user sees the outcome and a fast crash is not silent
    loop {
        std::thread::sleep(std::time::Duration::from_millis(300));
        let (running, error) = shared
            .read(|status| (status.game_running, status.error.clone()))
            .unwrap_or((false, None));
        if let Some(error) = error {
            bail!("{error}");
        }
        if !running {
            println!("vortex exited");
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
