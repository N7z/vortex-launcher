// SPDX-License-Identifier: AGPL-3.0-or-later
//! Background thread doing everything slow, so the UI never blocks.

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;

use anyhow::Result;

use crate::config::Config;
use crate::paths::Paths;
use crate::state::{Shared, Task};
use crate::{game, launch, net, proton};

pub enum Job {
    Detect,
    Setup,
    CheckUpdate,
    Play,
    SetSelfUpdate(bool),
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

    pub fn send(&self, job: Job) {
        // the worker only dies with the process, so a closed channel means we are shutting down
        self.jobs.send(job).ok();
    }
}

fn run(shared: Arc<Shared>, paths: Paths, inbox: Receiver<Job>) {
    let agent = net::agent();
    let mut config = Config::load(&paths.config_file());

    while let Ok(job) = inbox.recv() {
        let result = match job {
            Job::Detect => detect(&shared, &paths, &mut config),
            Job::Setup => setup(&agent, &shared, &paths, &mut config),
            Job::CheckUpdate => check_update(&agent, &shared, &paths, &mut config),
            Job::Play => play(&shared, &paths, &config),
            Job::SetSelfUpdate(value) => {
                config.allow_self_update = value;
                publish(&shared, &config);
                config.save(&paths.config_file())
            }
        };

        match result {
            Ok(()) => shared.finish(),
            Err(err) if net::is_cancelled(&err) => {
                shared.log("cancelled");
                shared.finish();
            }
            Err(err) => shared.fail(describe(&err)),
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

fn publish(shared: &Arc<Shared>, config: &Config) {
    let game_ready = config.game_ready();
    let proton_ready = config.proton_ready();
    let proton_name = config.proton.as_ref().map(|p| p.name.clone());
    let allow_self_update = config.allow_self_update;
    shared.update(|status| {
        status.game_ready = game_ready;
        status.proton_ready = proton_ready;
        status.proton_name = proton_name;
        status.allow_self_update = allow_self_update;
    });
}

fn detect(shared: &Arc<Shared>, paths: &Paths, config: &mut Config) -> Result<()> {
    shared.begin(Task::Detecting);
    paths.ensure()?;

    if !config.game_ready() {
        config.game = None;
    }
    if !config.proton_ready() {
        config.proton = proton::detect_managed(paths).or_else(proton::detect_system);
        if let Some(found) = &config.proton {
            shared.log(format!("found {}", found.name));
        }
    }
    publish(shared, config);
    config.save(&paths.config_file())
}

fn setup(agent: &ureq::Agent, shared: &Arc<Shared>, paths: &Paths, config: &mut Config) -> Result<()> {
    paths.ensure()?;

    if !config.proton_ready() {
        config.proton = proton::detect_managed(paths).or_else(proton::detect_system);
    }
    if !config.proton_ready() {
        shared.begin(Task::InstallingProton);
        config.proton = Some(proton::install(agent, paths, shared, shared.cancel_flag())?);
        config.save(&paths.config_file())?;
        publish(shared, config);
    }

    if !config.game_ready() {
        shared.begin(Task::InstallingGame);
        config.game = Some(game::install(agent, paths, shared, shared.cancel_flag())?);
        config.save(&paths.config_file())?;
    }

    publish(shared, config);
    shared.update(|status| status.update_available = false);
    Ok(())
}

fn check_update(
    agent: &ureq::Agent,
    shared: &Arc<Shared>,
    paths: &Paths,
    config: &mut Config,
) -> Result<()> {
    shared.begin(Task::CheckingUpdate);

    let Some(installed) = config.game.clone() else {
        return setup(agent, shared, paths, config);
    };

    let remote = game::probe(agent)?;
    let changed = remote.differs_from(
        installed.etag.as_deref(),
        installed.last_modified.as_deref(),
        installed.size,
    );
    if !changed {
        shared.log("vortex is up to date");
        shared.update(|status| status.update_available = false);
        return Ok(());
    }

    shared.log("new vortex build, downloading");
    shared.begin(Task::InstallingGame);
    config.game = Some(game::install(agent, paths, shared, shared.cancel_flag())?);
    config.save(&paths.config_file())?;
    publish(shared, config);
    shared.update(|status| status.update_available = false);
    Ok(())
}

fn play(shared: &Arc<Shared>, paths: &Paths, config: &Config) -> Result<()> {
    shared.begin(Task::Launching);
    launch::launch(paths, config, shared)
}
