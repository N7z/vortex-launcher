// SPDX-License-Identifier: AGPL-3.0-or-later
#![forbid(unsafe_code)]

#[cfg(feature = "gui")]
pub mod app;
pub mod auth;
pub mod config;
pub mod desktop;
pub mod game;
pub mod ipc;
pub mod launch;
pub mod logo;
pub mod net;
pub mod paths;
pub mod proton;
pub mod session;
pub mod shaders;
pub mod state;
pub mod worker;
