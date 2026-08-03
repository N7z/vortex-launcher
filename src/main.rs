// SPDX-License-Identifier: AGPL-3.0-or-later
#![forbid(unsafe_code)]

mod app;
mod config;
mod game;
mod launch;
mod net;
mod paths;
mod proton;
mod state;
mod worker;

use eframe::egui;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let paths = paths::Paths::discover()?;

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Vortex Launcher")
            .with_app_id("vortex-launcher")
            .with_inner_size([460.0, 420.0])
            .with_min_inner_size([420.0, 380.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Vortex Launcher",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, paths)))),
    )?;
    Ok(())
}
