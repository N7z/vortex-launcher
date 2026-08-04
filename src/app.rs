// SPDX-License-Identifier: AGPL-3.0-or-later
//! The window. Everything here is click-driven, no terminal step ever.

use std::sync::Arc;

use eframe::egui::{self, Color32, RichText};

use crate::paths::Paths;
use crate::state::{GameRow, Shared};
use crate::worker::{Job, Worker};

const CREDIT: &str = "Developed by zPaulinBRz | Logo by KitKat";

pub struct App {
    shared: Arc<Shared>,
    logo: Option<egui::TextureHandle>,
    worker: Worker,
    username: String,
    password: String,
    code: String,
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        paths: Paths,
        uri: Option<String>,
        listener: std::os::unix::net::UnixListener,
        desktop_note: Option<String>,
    ) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let logo = crate::logo::decode().map(|image| {
            let size = [image.width as usize, image.height as usize];
            let texture = egui::ColorImage::from_rgba_unmultiplied(size, &image.rgba);
            cc.egui_ctx.load_texture("logo", texture, egui::TextureOptions::LINEAR)
        });
        let shared = Shared::new(&paths.logs().join("launcher.log"));
        shared.attach(cc.egui_ctx.clone());
        if let Some(note) = desktop_note {
            shared.log(note);
        }

        let worker = Worker::spawn(Arc::clone(&shared), paths);
        worker.send(Job::Detect);
        if let Some(uri) = uri {
            worker.send(Job::PlayUri(uri));
        }

        let jobs = worker.handle();
        let ctx = cc.egui_ctx.clone();
        crate::ipc::serve(listener, move |message| {
            jobs.send(Job::PlayUri(message)).ok();
            ctx.request_repaint();
        });

        Self {
            shared,
            logo,
            worker,
            username: String::new(),
            password: String::new(),
            code: String::new(),
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let Some(snapshot) = self.shared.read(Snapshot::from_status) else {
            return;
        };

        egui::CentralPanel::default().show(ui, |ui| {
            ui.add_space(12.0);
            ui.vertical_centered(|ui| {
                if let Some(logo) = &self.logo {
                    ui.add(egui::Image::new(logo).fit_to_exact_size(egui::vec2(84.0, 84.0)));
                    ui.add_space(2.0);
                }
                ui.label(RichText::new("VORTEX").size(24.0).strong());
                ui.label(
                    RichText::new("unofficial linux launcher")
                        .size(11.0)
                        .color(Color32::from_gray(130)),
                );
            });
            ui.add_space(14.0);

            if let Some(error) = &snapshot.error {
                error_banner(ui, error, Color32::from_rgb(60, 26, 26), Color32::from_rgb(255, 180, 180));
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

        if snapshot.game_running || snapshot.task.is_some() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
        }
    }
}

impl App {
    fn idle_controls(&mut self, ui: &mut egui::Ui, snapshot: &Snapshot) {
        if !snapshot.game_ready || !snapshot.proton_ready {
            self.install_screen(ui);
            return;
        }
        if snapshot.account.is_none() {
            self.sign_in_screen(ui, snapshot);
            return;
        }
        self.play_screen(ui, snapshot);
    }

    fn install_screen(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            if big_button(ui, "Install").clicked() {
                self.worker.send(Job::Setup);
            }
            ui.add_space(10.0);
            ui.label(
                RichText::new("first run downloads Vortex and Proton, about 500 MB")
                    .size(11.0)
                    .color(Color32::from_gray(140)),
            );
        });
    }

    fn sign_in_screen(&mut self, ui: &mut egui::Ui, snapshot: &Snapshot) {
        if let Some(error) = &snapshot.auth_error {
            error_banner(ui, error, Color32::from_rgb(58, 40, 20), Color32::from_rgb(255, 214, 170));
            ui.add_space(8.0);
        }

        if snapshot.needs_2fa {
            ui.label(RichText::new("Two-factor code").strong());
            ui.add_space(4.0);
            let entered = ui
                .add(egui::TextEdit::singleline(&mut self.code).desired_width(f32::INFINITY))
                .lost_focus()
                && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.add_space(10.0);
            if (ui.button("Verify").clicked() || entered) && !self.code.trim().is_empty() {
                self.worker.send(Job::Submit2fa(self.code.trim().to_owned()));
                self.code.clear();
            }
            return;
        }

        ui.label(RichText::new("Sign in to play").strong());
        ui.add_space(8.0);
        ui.label(RichText::new("Username").size(11.0).color(Color32::from_gray(150)));
        ui.add(egui::TextEdit::singleline(&mut self.username).desired_width(f32::INFINITY));
        ui.add_space(6.0);
        ui.label(RichText::new("Password").size(11.0).color(Color32::from_gray(150)));
        let submitted = ui
            .add(
                egui::TextEdit::singleline(&mut self.password)
                    .password(true)
                    .desired_width(f32::INFINITY),
            )
            .lost_focus()
            && ui.input(|i| i.key_pressed(egui::Key::Enter));

        ui.add_space(12.0);
        let ready = !self.username.trim().is_empty() && !self.password.is_empty();
        let clicked = ui
            .add_enabled(ready, egui::Button::new("Sign in").min_size([120.0, 30.0].into()))
            .clicked();
        if ready && (clicked || submitted) {
            self.worker.send(Job::SignIn {
                username: self.username.trim().to_owned(),
                password: std::mem::take(&mut self.password),
            });
        }
    }

    fn play_screen(&mut self, ui: &mut egui::Ui, snapshot: &Snapshot) {
        ui.vertical_centered(|ui| {
            if snapshot.game_running {
                ui.add_enabled(false, egui::Button::new(RichText::new("Running").size(20.0)).min_size([230.0, 46.0].into()));
                ui.add_space(6.0);
                ui.label(RichText::new("Vortex is open").color(Color32::from_gray(150)));
            } else if big_button(ui, "Play").clicked() {
                self.worker.send(Job::Play);
            }
        });

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Games").size(11.0).color(Color32::from_gray(150)));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Refresh").clicked() {
                    self.worker.send(Job::LoadGames);
                }
            });
        });
        ui.add_space(4.0);

        egui::ScrollArea::vertical()
            .max_height(150.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                if snapshot.games.is_empty() {
                    ui.label(RichText::new("no games listed").color(Color32::from_gray(120)));
                }
                for game in &snapshot.games {
                    let selected = snapshot.selected_game == Some(game.id);
                    let label = format!("{} - {} playing", game.name, game.players);
                    if ui.selectable_label(selected, label).clicked() {
                        self.worker.send(Job::SelectGame(game.id));
                    }
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
                if snapshot.account.is_some() && ui.small_button("Sign out").clicked() {
                    self.worker.send(Job::SignOut);
                }
                if snapshot.game_ready && ui.small_button("Check for updates").clicked() {
                    self.worker.send(Job::CheckUpdate);
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

        let mut native_compiler = snapshot.native_shader_compiler;
        let toggled = ui
            .checkbox(&mut native_compiler, "Use Microsoft's shader compiler")
            .on_hover_text(
                "Fixes a black screen with working audio on some GPUs, where Proton's own \
                 shader compiler rejects the game's shaders. Downloads d3dcompiler_47 \
                 through winetricks the first time.",
            )
            .changed();
        if toggled {
            self.worker.send(Job::SetNativeShaderCompiler(native_compiler));
        }

        ui.add_space(8.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new(CREDIT).size(10.0).color(Color32::from_gray(95)));
        });
    }
}

fn big_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(egui::Button::new(RichText::new(label).size(20.0)).min_size([230.0, 46.0].into()))
}

fn error_banner(ui: &mut egui::Ui, text: &str, fill: Color32, ink: Color32) {
    egui::Frame::new()
        .fill(fill)
        .inner_margin(10.0)
        .corner_radius(6.0)
        .show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add(egui::Label::new(RichText::new(text).color(ink)).halign(egui::Align::Center));
            });
        });
}

fn status_line(snapshot: &Snapshot) -> String {
    let who = snapshot.account.clone().unwrap_or_else(|| "signed out".into());
    let proton = match (&snapshot.proton_name, snapshot.proton_ready) {
        (Some(name), true) => name.clone(),
        _ => "Proton missing".into(),
    };
    format!("{who} - {proton}")
}

/// copy of the shared status, so the lock is never held while drawing
struct Snapshot {
    task: Option<&'static str>,
    detail: String,
    progress: Option<f32>,
    error: Option<String>,
    game_ready: bool,
    proton_ready: bool,
    proton_name: Option<String>,
    game_running: bool,
    allow_self_update: bool,
    native_shader_compiler: bool,
    account: Option<String>,
    needs_2fa: bool,
    auth_error: Option<String>,
    games: Vec<GameRow>,
    selected_game: Option<u64>,
}

impl Snapshot {
    fn from_status(status: &crate::state::Status) -> Self {
        Self {
            task: status.task.map(|t| t.label()),
            detail: status.detail.clone(),
            progress: status.progress,
            error: status.error.clone(),
            game_ready: status.game_ready,
            proton_ready: status.proton_ready,
            proton_name: status.proton_name.clone(),
            game_running: status.game_running,
            allow_self_update: status.allow_self_update,
            native_shader_compiler: status.native_shader_compiler,
            account: status.account.clone(),
            needs_2fa: status.needs_2fa,
            auth_error: status.auth_error.clone(),
            games: status.games.clone(),
            selected_game: status.selected_game,
        }
    }
}
