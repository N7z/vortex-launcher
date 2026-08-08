// SPDX-License-Identifier: AGPL-3.0-or-later
//! Background thread doing everything slow, so the UI never blocks.

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;

use anyhow::{bail, Result};

use crate::config::Config;
use crate::paths::Paths;
use crate::session::Session;
use crate::state::{GameRow, Shared, Task};
use crate::{auth, game, launch, net, proton, session, studio};

pub enum Job {
    Detect,
    Setup,
    CheckUpdate,
    Play,
    SetSelfUpdate(bool),
    SetNativeShaderCompiler(bool),
    SignIn { username: String, password: String },
    Submit2fa(String),
    SignOut,
    LoadGames,
    SelectGame(u64),
    PlayUri(String),
    OpenStudio,
    StudioUri(String),
}

pub struct Worker {
    jobs: Sender<Job>,
}

impl Worker {
    pub fn spawn(shared: Arc<Shared>, paths: Paths) -> Self {
        let (jobs, inbox) = mpsc::channel();
        std::thread::spawn(move || run(shared, paths, inbox));
        Self { jobs }
    }

    pub fn handle(&self) -> Sender<Job> {
        self.jobs.clone()
    }

    pub fn send(&self, job: Job) {
        // the worker only dies with the process, so a closed channel means we are shutting down
        self.jobs.send(job).ok();
    }
}

struct Ctx {
    shared: Arc<Shared>,
    paths: Paths,
    config: Config,
    session: Option<Session>,
    pending_2fa: Option<String>,
    /// what the user typed, kept because the account lookup can fail after a good login
    typed_username: String,
    selected_game: Option<u64>,
    /// a link that arrived before the game was installed
    pending_uri: Option<String>,
}

fn run(shared: Arc<Shared>, paths: Paths, inbox: Receiver<Job>) {
    let agent = net::agent();
    let config = Config::load(&paths.config_file());
    let session = session::load(&paths.session_file());
    let mut ctx = Ctx {
        shared,
        paths,
        config,
        session,
        pending_2fa: None,
        typed_username: String::new(),
        selected_game: None,
        pending_uri: None,
    };

    while let Ok(job) = inbox.recv() {
        let result = match job {
            Job::Detect => detect(&mut ctx),
            Job::Setup => setup(&agent, &mut ctx),
            Job::CheckUpdate => check_update(&agent, &mut ctx),
            Job::Play => play(&mut ctx),
            Job::SetSelfUpdate(value) => {
                ctx.config.allow_self_update = value;
                publish(&ctx);
                ctx.config.save(&ctx.paths.config_file())
            }
            Job::SetNativeShaderCompiler(value) => set_native_shader_compiler(&mut ctx, value),
            Job::SignIn { username, password } => sign_in(&mut ctx, &username, &password),
            Job::Submit2fa(code) => submit_2fa(&mut ctx, &code),
            Job::SignOut => sign_out(&mut ctx),
            Job::LoadGames => load_games(&ctx),
            Job::PlayUri(uri) => play_uri(&mut ctx, uri),
            Job::OpenStudio => open_studio(&agent, &mut ctx),
            Job::StudioUri(uri) => studio_uri(&mut ctx, uri),
            Job::SelectGame(id) => {
                ctx.selected_game = Some(id);
                ctx.shared.update(|status| status.selected_game = Some(id));
                Ok(())
            }
        };

        match result {
            Ok(()) => ctx.shared.finish(),
            Err(err) if net::is_cancelled(&err) => {
                ctx.shared.log("cancelled");
                ctx.shared.finish();
            }
            Err(err) => ctx.shared.fail(describe(&err)),
        }
    }
}

/// full anyhow chain on one line, so the UI shows cause and not just the top
fn describe(err: &anyhow::Error) -> String {
    let mut parts = err.chain().map(|cause| cause.to_string());
    let mut text = parts.next().unwrap_or_else(|| "unknown error".into());
    for cause in parts {
        text.push_str(": ");
        text.push_str(&cause);
    }
    text
}

fn expired(err: &anyhow::Error) -> bool {
    err.chain()
        .any(|cause| cause.to_string().starts_with("session expired"))
}

fn publish(ctx: &Ctx) {
    let game_ready = ctx.config.game_ready();
    let studio_ready = ctx.config.studio_ready();
    let proton_ready = ctx.config.proton_ready();
    let proton_name = ctx.config.proton.as_ref().map(|p| p.name.clone());
    let allow_self_update = ctx.config.allow_self_update;
    let native_shader_compiler = ctx.config.native_shader_compiler;
    let account = ctx.session.as_ref().map(|s| s.username.clone());
    ctx.shared.update(|status| {
        status.game_ready = game_ready;
        status.studio_ready = studio_ready;
        status.proton_ready = proton_ready;
        status.proton_name = proton_name;
        status.allow_self_update = allow_self_update;
        status.native_shader_compiler = native_shader_compiler;
        status.account = account;
    });
}

fn open_studio(agent: &ureq::Agent, ctx: &mut Ctx) -> Result<()> {
    let Some(session) = ctx.session.clone() else {
        bail!("sign in first, the studio download needs your account");
    };

    if !ctx.config.studio_ready() {
        ctx.config.studio = None;
        ctx.shared.begin(Task::InstallingStudio);
        ctx.config.studio = Some(studio::install(
            agent,
            &ctx.paths,
            &ctx.shared,
            ctx.shared.cancel_flag(),
            &session.token,
        )?);
        ctx.config.save(&ctx.paths.config_file())?;
        publish(ctx);
    }

    ctx.shared.begin(Task::LaunchingStudio);
    ctx.shared
        .update(|status| status.detail = "asking the server for a launch link".into());

    let uri = match auth::studio_uri(&session.token) {
        Ok(uri) => uri,
        Err(err) if expired(&err) => {
            session::clear(&ctx.paths.session_file());
            ctx.session = None;
            publish(ctx);
            bail!("your session expired, sign in again");
        }
        Err(err) => return Err(err),
    };

    launch::launch_target(
        &ctx.paths,
        &ctx.config,
        &ctx.shared,
        launch::Target::Studio,
        Some(&uri),
    )
}

fn studio_uri(ctx: &mut Ctx, uri: String) -> Result<()> {
    if !auth::is_studio_uri(&uri) {
        bail!("that link is not a vortex-studio:// link");
    }
    if !ctx.config.studio_ready() || !ctx.config.proton_ready() {
        ctx.shared
            .log("studio link received, but the studio is not installed yet");
        bail!("open the Studio from the launcher once to install it, then the link will work");
    }

    ctx.shared.begin(Task::LaunchingStudio);
    ctx.shared.log("opening a studio link from the browser");
    launch::launch_target(
        &ctx.paths,
        &ctx.config,
        &ctx.shared,
        launch::Target::Studio,
        Some(&uri),
    )
}

fn detect(ctx: &mut Ctx) -> Result<()> {
    ctx.shared.begin(Task::Detecting);
    ctx.paths.ensure()?;

    // a recreated prefix takes the DLL with it, and an override pointing at
    // nothing would leave the checkbox claiming a fix that is not in effect
    if ctx.config.native_shader_compiler && !crate::shaders::is_installed(&ctx.paths) {
        ctx.config.native_shader_compiler = false;
        ctx.shared
            .log("native d3dcompiler_47 is not in the prefix, turning the option back off");
    }

    if !ctx.config.game_ready() {
        ctx.config.game = None;
    }
    if !ctx.config.studio_ready() {
        ctx.config.studio = None;
    }
    if !ctx.config.proton_ready() {
        ctx.config.proton = proton::detect_managed(&ctx.paths).or_else(proton::detect_system);
        if let Some(found) = &ctx.config.proton {
            ctx.shared.log(format!("found {}", found.name));
        }
    }
    publish(ctx);
    ctx.config.save(&ctx.paths.config_file())?;

    if let Some(stored) = ctx.session.clone() {
        match auth::account(&stored.token) {
            Ok(account) => {
                ctx.shared.log(format!("signed in as {}", account.username));
                ctx.session = Some(Session {
                    token: stored.token,
                    username: account.username,
                    user_id: account.id,
                });
                publish(ctx);
                return load_games(ctx);
            }
            // neither a 401 here nor being offline is proof the token is dead.
            // the play page decides, and it clears the session when it refuses.
            Err(err) => {
                ctx.shared
                    .log(format!("account check failed: {}", describe(&err)));
                publish(ctx);
                return load_games(ctx);
            }
        }
    }
    Ok(())
}

fn setup(agent: &ureq::Agent, ctx: &mut Ctx) -> Result<()> {
    ctx.paths.ensure()?;

    if !ctx.config.proton_ready() {
        ctx.config.proton = proton::detect_managed(&ctx.paths).or_else(proton::detect_system);
    }
    if !ctx.config.proton_ready() {
        ctx.shared.begin(Task::InstallingProton);
        ctx.config.proton = Some(proton::install(
            agent,
            &ctx.paths,
            &ctx.shared,
            ctx.shared.cancel_flag(),
        )?);
        ctx.config.save(&ctx.paths.config_file())?;
        publish(ctx);
    }

    if !ctx.config.game_ready() {
        ctx.shared.begin(Task::InstallingGame);
        ctx.config.game = Some(game::install(
            agent,
            &ctx.paths,
            &ctx.shared,
            ctx.shared.cancel_flag(),
        )?);
        ctx.config.save(&ctx.paths.config_file())?;
    }

    publish(ctx);
    ctx.shared.update(|status| status.update_available = false);

    if let Some(uri) = ctx.pending_uri.take() {
        return play_uri(ctx, uri);
    }
    Ok(())
}

fn check_update(agent: &ureq::Agent, ctx: &mut Ctx) -> Result<()> {
    ctx.shared.begin(Task::CheckingUpdate);

    let Some(installed) = ctx.config.game.clone() else {
        return setup(agent, ctx);
    };

    let remote = game::probe(agent)?;
    let changed = remote.differs_from(
        installed.etag.as_deref(),
        installed.last_modified.as_deref(),
        installed.size,
    );
    if !changed {
        ctx.shared.log("vortex is up to date");
        ctx.shared.update(|status| status.update_available = false);
        return Ok(());
    }

    ctx.shared.log("new vortex build, downloading");
    ctx.shared.begin(Task::InstallingGame);
    ctx.config.game = Some(game::install(
        agent,
        &ctx.paths,
        &ctx.shared,
        ctx.shared.cancel_flag(),
    )?);
    ctx.config.save(&ctx.paths.config_file())?;
    publish(ctx);
    ctx.shared.update(|status| status.update_available = false);
    Ok(())
}

/// turning this on has to fetch the DLL first, and a failed fetch must leave the
/// checkbox unticked rather than promising an override that will not resolve
fn set_native_shader_compiler(ctx: &mut Ctx, value: bool) -> Result<()> {
    ctx.config.native_shader_compiler = value;
    publish(ctx);

    if value && !crate::shaders::is_installed(&ctx.paths) {
        ctx.shared.begin(Task::InstallingCompiler);
        if let Err(err) = crate::shaders::install(&ctx.paths, &ctx.config, &ctx.shared) {
            ctx.config.native_shader_compiler = false;
            publish(ctx);
            ctx.config.save(&ctx.paths.config_file()).ok();
            return Err(err.context("could not install the shader compiler, the option stayed off"));
        }
    }
    ctx.config.save(&ctx.paths.config_file())
}

fn sign_in(ctx: &mut Ctx, username: &str, password: &str) -> Result<()> {
    ctx.shared.begin(Task::SigningIn);
    ctx.shared.update(|status| status.auth_error = None);
    ctx.typed_username = username.to_owned();

    match auth::login(username, password) {
        Ok(auth::Login::Done(token)) => finish_sign_in(ctx, token),
        Ok(auth::Login::Needs2fa(pending)) => {
            ctx.pending_2fa = Some(pending);
            ctx.shared.update(|status| status.needs_2fa = true);
            Ok(())
        }
        Err(err) => auth_failed(ctx, &err),
    }
}

fn submit_2fa(ctx: &mut Ctx, code: &str) -> Result<()> {
    ctx.shared.begin(Task::SigningIn);
    let Some(pending) = ctx.pending_2fa.clone() else {
        bail!("the two-factor step expired, sign in again");
    };

    match auth::login_2fa(&pending, code) {
        Ok(auth::Login::Done(token)) => {
            ctx.pending_2fa = None;
            ctx.shared.update(|status| status.needs_2fa = false);
            finish_sign_in(ctx, token)
        }
        Ok(auth::Login::Needs2fa(_)) => {
            ctx.shared
                .update(|status| status.auth_error = Some("that code was not accepted".into()));
            Ok(())
        }
        Err(err) => auth_failed(ctx, &err),
    }
}

/// a rejected login is the server talking to the user, not a launcher failure
fn auth_failed(ctx: &Ctx, err: &anyhow::Error) -> Result<()> {
    let message = describe(err);
    ctx.shared.log(format!("sign in refused: {message}"));
    ctx.shared.update(|status| status.auth_error = Some(message));
    Ok(())
}

fn finish_sign_in(ctx: &mut Ctx, token: String) -> Result<()> {
    // the login already succeeded, so a failing account lookup must not undo it
    let (username, user_id) = match auth::account(&token) {
        Ok(account) => (account.username, account.id),
        Err(err) => {
            ctx.shared
                .log(format!("account lookup failed: {}", describe(&err)));
            (ctx.typed_username.clone(), 0)
        }
    };

    let session = Session {
        token,
        username: username.clone(),
        user_id,
    };
    session::save(&ctx.paths.session_file(), &session)?;
    ctx.session = Some(session);
    ctx.shared.update(|status| {
        status.auth_error = None;
        status.needs_2fa = false;
    });
    ctx.shared.log(format!("signed in as {username}"));
    publish(ctx);
    load_games(ctx)
}

fn sign_out(ctx: &mut Ctx) -> Result<()> {
    session::clear(&ctx.paths.session_file());
    ctx.session = None;
    ctx.pending_2fa = None;
    ctx.selected_game = None;
    ctx.shared.update(|status| {
        status.account = None;
        status.needs_2fa = false;
        status.auth_error = None;
        status.games.clear();
        status.selected_game = None;
    });
    ctx.shared.log("signed out");
    Ok(())
}

fn load_games(ctx: &Ctx) -> Result<()> {
    ctx.shared.begin(Task::LoadingGames);
    let games = auth::games()?;
    let mut rows: Vec<GameRow> = games
        .into_iter()
        .map(|g| GameRow {
            id: g.id,
            name: g.name,
            players: g.players,
        })
        .collect();
    rows.sort_by(|a, b| b.players.cmp(&a.players).then_with(|| a.name.cmp(&b.name)));
    ctx.shared.update(|status| {
        if status.selected_game.is_none() {
            status.selected_game = rows.first().map(|row| row.id);
        }
        status.games = rows;
    });
    Ok(())
}

fn play(ctx: &mut Ctx) -> Result<()> {
    ctx.shared.begin(Task::Launching);

    let Some(session) = ctx.session.clone() else {
        bail!("sign in first");
    };
    let game_id = ctx
        .selected_game
        .or_else(|| ctx.shared.read(|status| status.selected_game).flatten())
        .unwrap_or(1);

    ctx.shared
        .update(|status| status.detail = "asking the server for a launch link".into());

    let uri = match auth::play_uri(&session.token, game_id, None) {
        Ok(uri) => uri,
        Err(err) if expired(&err) => {
            session::clear(&ctx.paths.session_file());
            ctx.session = None;
            publish(ctx);
            bail!("your session expired, sign in again");
        }
        Err(err) => return Err(err),
    };

    launch::launch(&ctx.paths, &ctx.config, &ctx.shared, Some(&uri))
}

fn play_uri(ctx: &mut Ctx, uri: String) -> Result<()> {
    if !auth::is_launch_uri(&uri) {
        bail!("that link is not a vortex:// launch link");
    }
    if !ctx.config.game_ready() || !ctx.config.proton_ready() {
        ctx.shared.log("link received, but nothing is installed yet");
        ctx.pending_uri = Some(uri);
        bail!("install Vortex first, then the link will open");
    }

    ctx.shared.begin(Task::Launching);
    ctx.shared.log("opening a link from the browser");
    launch::launch(&ctx.paths, &ctx.config, &ctx.shared, Some(&uri))
}
