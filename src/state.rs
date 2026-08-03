// SPDX-License-Identifier: AGPL-3.0-or-later
//! State shared between the UI thread and the worker thread.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const LOG_LINES: usize = 400;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Task {
    Detecting,
    InstallingGame,
    InstallingProton,
    CheckingUpdate,
    Launching,
}

impl Task {
    pub fn label(self) -> &'static str {
        match self {
            Task::Detecting => "Checking your installation",
            Task::InstallingGame => "Downloading Vortex",
            Task::InstallingProton => "Downloading Proton",
            Task::CheckingUpdate => "Checking for updates",
            Task::Launching => "Starting Vortex",
        }
    }
}

#[derive(Default)]
pub struct Status {
    pub task: Option<Task>,
    pub detail: String,
    pub progress: Option<f32>,
    pub error: Option<String>,
    pub log: VecDeque<String>,
    pub game_ready: bool,
    pub proton_ready: bool,
    pub proton_name: Option<String>,
    pub game_running: bool,
    pub allow_self_update: bool,
    pub update_available: bool,
}

pub struct Shared {
    status: Mutex<Status>,
    cancel: AtomicBool,
    ctx: Mutex<Option<eframe::egui::Context>>,
}

impl Shared {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            status: Mutex::new(Status::default()),
            cancel: AtomicBool::new(false),
            ctx: Mutex::new(None),
        })
    }

    pub fn attach(&self, ctx: eframe::egui::Context) {
        if let Ok(mut slot) = self.ctx.lock() {
            *slot = Some(ctx);
        }
    }

    // a poisoned lock means a worker panicked, keep the UI alive on the last good state
    pub fn read<T>(&self, f: impl FnOnce(&Status) -> T) -> Option<T> {
        self.status.lock().ok().map(|status| f(&status))
    }

    pub fn update(&self, f: impl FnOnce(&mut Status)) {
        if let Ok(mut status) = self.status.lock() {
            f(&mut status);
        }
        self.repaint();
    }

    pub fn log(&self, line: impl Into<String>) {
        let line = line.into();
        if let Ok(mut status) = self.status.lock() {
            if status.log.len() >= LOG_LINES {
                status.log.pop_front();
            }
            status.log.push_back(line);
        }
        self.repaint();
    }

    pub fn begin(&self, task: Task) {
        self.cancel.store(false, Ordering::Relaxed);
        self.update(|status| {
            status.task = Some(task);
            status.detail.clear();
            status.progress = None;
            status.error = None;
        });
    }

    pub fn finish(&self) {
        self.update(|status| {
            status.task = None;
            status.detail.clear();
            status.progress = None;
        });
    }

    pub fn fail(&self, message: impl Into<String>) {
        let message = message.into();
        self.log(format!("error: {message}"));
        self.update(|status| {
            status.task = None;
            status.progress = None;
            status.detail.clear();
            status.error = Some(message);
        });
    }

    pub fn request_cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn cancel_flag(&self) -> &AtomicBool {
        &self.cancel
    }

    fn repaint(&self) {
        if let Ok(ctx) = self.ctx.lock() {
            if let Some(ctx) = ctx.as_ref() {
                ctx.request_repaint();
            }
        }
    }
}
