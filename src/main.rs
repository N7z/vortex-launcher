// SPDX-License-Identifier: AGPL-3.0-or-later
#![forbid(unsafe_code)]

mod app;
mod auth;
mod config;
mod game;
mod launch;
mod net;
mod paths;
mod proton;
mod session;
mod state;
mod worker;

use eframe::egui;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let paths = paths::Paths::discover()?;

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Vortex Launcher")
            .with_app_id("vortex-launcher")
            .with_inner_size([460.0, 560.0])
            .with_min_inner_size([420.0, 460.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Vortex Launcher",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, paths)))),
    )?;
    Ok(())
}
