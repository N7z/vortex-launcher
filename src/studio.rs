// SPDX-License-Identifier: AGPL-3.0-or-later
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::Result;

use crate::config::GameInstall;
use crate::game::{self, Build};
use crate::net::Remote;
use crate::paths::Paths;
use crate::state::Shared;

pub const DOWNLOAD_URL: &str = "https://playvortex.io/download/studio-windows";
const ARCHIVE: &str = "VortexStudio-Windows.zip";
const EXE: &str = "VortexStudio.exe";

fn build<'a>(paths: &Paths, token: &'a str) -> Build<'a> {
    Build {
        url: DOWNLOAD_URL,
        token: Some(token),
        archive: ARCHIVE,
        exe: EXE,
        label: "studio",
        dir: paths.studio(),
    }
}

pub fn probe(agent: &ureq::Agent, token: &str) -> Result<Remote> {
    crate::net::probe_as(agent, DOWNLOAD_URL, Some(token))
}

pub fn install(
    agent: &ureq::Agent,
    paths: &Paths,
    shared: &Arc<Shared>,
    cancel: &AtomicBool,
    token: &str,
) -> Result<GameInstall> {
    game::install_build(agent, &build(paths, token), paths, shared, cancel)
}
