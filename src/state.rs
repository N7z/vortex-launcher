// SPDX-License-Identifier: AGPL-3.0-or-later
//! State shared between the UI thread and the worker thread.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Task {
    Detecting,
    InstallingGame,
    InstallingStudio,
    InstallingProton,
    CheckingUpdate,
    Launching,
    LaunchingStudio,
    SigningIn,
    LoadingGames,
    InstallingCompiler,
}

impl Task {
    pub fn label(self) -> &'static str {
        match self {
            Task::Detecting => "Checking your installation",
            Task::InstallingGame => "Downloading Vortex",
            Task::InstallingStudio => "Downloading Vortex Studio",
            Task::InstallingProton => "Downloading Proton",
            Task::CheckingUpdate => "Checking for updates",
            Task::Launching => "Starting Vortex",
            Task::LaunchingStudio => "Starting Vortex Studio",
            Task::SigningIn => "Signing in",
            Task::LoadingGames => "Loading games",
            Task::InstallingCompiler => "Installing the shader compiler",
        }
    }
}

#[derive(Clone, Debug)]
pub struct GameRow {
    pub id: u64,
    pub name: String,
    pub players: u64,
}

/// one entry of the Proton dropdown
#[derive(Clone, Debug)]
pub struct ProtonRow {
    pub name: String,
    /// None for a release that is not downloaded yet
    pub dir: Option<std::path::PathBuf>,
    /// borrowed from a steam install rather than downloaded by the launcher
    pub system: bool,
    /// false when the build wants a newer glibc than this system has.
    /// always true for a release not on disk, there is nothing to inspect
    pub runs_here: bool,
}

#[derive(Default)]
pub struct Status {
    pub task: Option<Task>,
    pub detail: String,
    pub progress: Option<f32>,
    pub error: Option<String>,
    pub game_ready: bool,
    pub studio_ready: bool,
    pub studio_running: bool,
    pub proton_ready: bool,
    pub proton_name: Option<String>,
    pub proton_dir: Option<std::path::PathBuf>,
    /// the chosen build cannot be loaded here, the game would die on launch
    pub proton_mismatch: bool,
    /// a release picked from the list but not downloaded yet
    pub proton_wanted: Option<String>,
    pub proton_options: Vec<ProtonRow>,
    pub game_running: bool,
    pub allow_self_update: bool,
    pub native_shader_compiler: bool,
    pub update_available: bool,
    pub account: Option<String>,
    pub needs_2fa: bool,
    pub auth_error: Option<String>,
    pub games: Vec<GameRow>,
    pub selected_game: Option<u64>,
}

pub struct Shared {
    status: Mutex<Status>,
    cancel: AtomicBool,
    repaint: Mutex<Option<Box<dyn Fn() + Send>>>,
    log: Mutex<Option<std::fs::File>>,
}

impl Shared {
    pub fn new(log_path: &std::path::Path) -> Arc<Self> {
        if let Some(parent) = log_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        Arc::new(Self {
            status: Mutex::new(Status::default()),
            cancel: AtomicBool::new(false),
            repaint: Mutex::new(None),
            log: Mutex::new(std::fs::File::create(log_path).ok()),
        })
    }

    /// hook the UI's repaint in; headless callers just never attach one
    pub fn attach(&self, repaint: impl Fn() + Send + 'static) {
        if let Ok(mut slot) = self.repaint.lock() {
            *slot = Some(Box::new(repaint));
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
        use std::io::Write;

        if let Ok(mut file) = self.log.lock() {
            if let Some(file) = file.as_mut() {
                writeln!(file, "{}", line.into()).ok();
                file.flush().ok();
            }
        }
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
        if let Ok(repaint) = self.repaint.lock() {
            if let Some(repaint) = repaint.as_ref() {
                repaint();
            }
        }
    }
}
