// SPDX-License-Identifier: AGPL-3.0-or-later
//! The window. Everything here is click-driven, no terminal step ever.

use std::sync::Arc;

use eframe::egui::{self, Color32, RichText};

use crate::paths::Paths;
use crate::state::Shared;
use crate::worker::{Job, Worker};

pub struct App {
    shared: Arc<Shared>,
    worker: Worker,
    show_log: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, paths: Paths) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let shared = Shared::new();
        shared.attach(cc.egui_ctx.clone());

        let worker = Worker::spawn(Arc::clone(&shared), paths);
        worker.send(Job::Detect);

        Self {
            shared,
            worker,
            show_log: false,
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let Some(snapshot) = self.shared.read(Snapshot::from_status) else {
            return;
        };

        egui::CentralPanel::default().show(ui, |ui| {
            ui.add_space(14.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("VORTEX").size(30.0).strong());
                ui.label(
                    RichText::new("unofficial linux launcher")
                        .size(11.0)
                        .color(Color32::from_gray(130)),
                );
            });
            ui.add_space(16.0);

            if let Some(error) = &snapshot.error {
                error_banner(ui, error);
                ui.add_space(10.0);
            }

            if let Some(task) = snapshot.task {
                ui.label(RichText::new(task).strong());
                match snapshot.progress {
                    Some(fraction) => {
                        ui.add(egui::ProgressBar::new(fraction).show_percentage().animate(true));
                    }
                    None => {
                        ui.add(egui::ProgressBar::new(0.0).animate(true));
                    }
                }
                if !snapshot.detail.is_empty() {
                    ui.label(RichText::new(&snapshot.detail).color(Color32::from_gray(150)));
                }
                ui.add_space(8.0);
                if ui.button("Cancel").clicked() {
                    self.shared.request_cancel();
                }
            } else {
                self.idle_controls(ui, &snapshot);
            }

            ui.add_space(12.0);
            ui.separator();
            self.footer(ui, &snapshot);
        });

        // the log pane needs a steady repaint while output is streaming in
        if snapshot.game_running || snapshot.task.is_some() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
        }
    }
}

impl App {
    fn idle_controls(&mut self, ui: &mut egui::Ui, snapshot: &Snapshot) {
        let ready = snapshot.game_ready && snapshot.proton_ready;

        ui.vertical_centered(|ui| {
            if snapshot.game_running {
                ui.add_enabled(
                    false,
                    egui::Button::new(RichText::new("Running").size(20.0)).min_size([230.0, 46.0].into()),
                );
                ui.add_space(6.0);
                ui.label(RichText::new("Vortex is open").color(Color32::from_gray(150)));
                return;
            }

            let label = if ready { "Play" } else { "Install" };
            if ui
                .add(egui::Button::new(RichText::new(label).size(20.0)).min_size([230.0, 46.0].into()))
                .clicked()
            {
                self.worker.send(if ready { Job::Play } else { Job::Setup });
            }

            ui.add_space(10.0);
            if ready && ui.button("Check for updates").clicked() {
                self.worker.send(Job::CheckUpdate);
            }
        });
    }

    fn footer(&mut self, ui: &mut egui::Ui, snapshot: &Snapshot) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(status_line(snapshot))
                    .size(11.0)
                    .color(Color32::from_gray(140)),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let toggle = if self.show_log { "Hide details" } else { "Details" };
                if ui.small_button(toggle).clicked() {
                    self.show_log = !self.show_log;
                }
            });
        });

        let mut self_update = snapshot.allow_self_update;
        if ui
            .checkbox(&mut self_update, "Let Vortex update itself on start")
            .changed()
        {
            self.worker.send(Job::SetSelfUpdate(self_update));
        }

        if self.show_log {
            ui.add_space(6.0);
            egui::ScrollArea::vertical()
                .max_height(160.0)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for line in &snapshot.log {
                        ui.label(RichText::new(line).monospace().size(11.0));
                    }
                });
        }
    }
}

fn error_banner(ui: &mut egui::Ui, error: &str) {
    egui::Frame::new()
        .fill(Color32::from_rgb(60, 26, 26))
        .inner_margin(10.0)
        .corner_radius(6.0)
        .show(ui, |ui| {
            ui.label(RichText::new(error).color(Color32::from_rgb(255, 180, 180)));
        });
}

fn status_line(snapshot: &Snapshot) -> String {
    let game = if snapshot.game_ready { "Vortex ok" } else { "Vortex missing" };
    let proton = match (&snapshot.proton_name, snapshot.proton_ready) {
        (Some(name), true) => name.clone(),
        _ => "Proton missing".into(),
    };
    format!("{game} · {proton}")
}

/// copy of the shared status, so the lock is never held while drawing
struct Snapshot {
    task: Option<&'static str>,
    detail: String,
    progress: Option<f32>,
    error: Option<String>,
    log: Vec<String>,
    game_ready: bool,
    proton_ready: bool,
    proton_name: Option<String>,
    game_running: bool,
    allow_self_update: bool,
}

impl Snapshot {
    fn from_status(status: &crate::state::Status) -> Self {
        Self {
            task: status.task.map(|t| t.label()),
            detail: status.detail.clone(),
            progress: status.progress,
            error: status.error.clone(),
            log: status.log.iter().cloned().collect(),
            game_ready: status.game_ready,
            proton_ready: status.proton_ready,
            proton_name: status.proton_name.clone(),
            game_running: status.game_running,
            allow_self_update: status.allow_self_update,
        }
    }
}
