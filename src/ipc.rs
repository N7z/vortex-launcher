// SPDX-License-Identifier: AGPL-3.0-or-later
//! One socket, so a vortex:// link opens in the window that is already running.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

const SOCKET: &str = "vortex-launcher.sock";

pub fn socket_path() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .or_else(dirs::data_dir)
        .unwrap_or_else(std::env::temp_dir)
        .join(SOCKET)
}

/// hands the message to a running launcher, false when there is none
pub fn send(message: &str) -> bool {
    let Ok(mut stream) = UnixStream::connect(socket_path()) else {
        return false;
    };
    writeln!(stream, "{message}").is_ok()
}

/// takes the socket, or None when another launcher already holds it
pub fn listen() -> Option<UnixListener> {
    let path = socket_path();
    match UnixListener::bind(&path) {
        Ok(listener) => Some(listener),
        // a socket nobody answers is left over from a crash, so it is ours to replace
        Err(_) if !send("") => {
            std::fs::remove_file(&path).ok();
            UnixListener::bind(&path).ok()
        }
        Err(_) => None,
    }
}

pub fn serve(listener: UnixListener, on_message: impl Fn(String) + Send + 'static) {
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut line = String::new();
            if BufReader::new(stream).read_line(&mut line).is_ok() {
                let line = line.trim().to_owned();
                if !line.is_empty() {
                    on_message(line);
                }
            }
        }
    });
}
