// SPDX-License-Identifier: AGPL-3.0-or-later
#![forbid(unsafe_code)]

mod app;
mod auth;
mod config;
mod desktop;
mod game;
mod ipc;
mod launch;
mod logo;
mod net;
mod paths;
mod proton;
mod session;
mod shaders;
mod state;
mod worker;

use eframe::egui;

fn icon() -> egui::IconData {
    match logo::decode() {
        Some(image) => egui::IconData {
            rgba: image.rgba,
            width: image.width,
            height: image.height,
        },
        None => egui::IconData::default(),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let paths = paths::Paths::discover()?;
    let uri = std::env::args().nth(1).filter(|arg| auth::is_launch_uri(arg));

    // a link from the browser belongs in the window that is already open
    let Some(listener) = ipc::listen() else {
        ipc::send(uri.as_deref().unwrap_or(""));
        return Ok(());
    };

    // has to happen before the window exists: the desktop shell resolves our app id to a
    // desktop entry the moment the window is mapped, and never looks again for that window
    let desktop_note = match desktop::install() {
        Ok(true) => Some("registered as the vortex:// handler".to_string()),
        Ok(false) => None,
        Err(err) => Some(format!("desktop entry failed: {err:#}")),
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Vortex Launcher")
            .with_app_id("vortex-launcher")
            .with_icon(icon())
            .with_inner_size([460.0, 560.0])
            .with_min_inner_size([420.0, 460.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Vortex Launcher",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, paths, uri, listener, desktop_note)))),
    )?;
    Ok(())
}
