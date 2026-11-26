//! Desktop notifications through `notify-send`.
//!
//! These run on a worker thread, off the key-release path. `notify-send` is
//! spawned rather than speaking D-Bus in-process: a native client would add
//! either libdbus or an async D-Bus stack for three notifications per
//! dictation, and with the DMS indicator most people turn them off.

use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Sender};
use std::thread::JoinHandle;

struct Job {
    text: String,
    timeout_ms: u32,
}

pub struct Notifier {
    tx: Option<Sender<Job>>,
    thread: Option<JoinHandle<()>>,
}

impl Notifier {
    /// A notifier that does nothing when `enabled` is false.
    pub fn new(enabled: bool) -> Self {
        if !enabled {
            return Self { tx: None, thread: None };
        }
        let (tx, rx) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            // Each notification replaces the previous one.
            let mut id: Option<String> = None;
            for Job { text, timeout_ms } in rx {
                let mut cmd = Command::new("notify-send");
                cmd.args(["-a", "walkie", "-u", "low", "-p", "-t", &timeout_ms.to_string()]);
                if let Some(id) = &id {
                    cmd.args(["-r", id]);
                }
                cmd.arg("--").arg(&text).stdin(Stdio::null()).stderr(Stdio::null());
                if let Ok(out) = cmd.output() {
                    let new = String::from_utf8_lossy(&out.stdout).trim().to_owned();
                    id = (!new.is_empty()).then_some(new);
                }
            }
        });
        Self { tx: Some(tx), thread: Some(thread) }
    }

    /// Shows `text`; `timeout_ms` 0 keeps it until replaced.
    pub fn show(&self, text: impl Into<String>, timeout_ms: u32) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Job { text: text.into(), timeout_ms });
        }
    }
}

impl Drop for Notifier {
    /// Waits for queued notifications to go out.
    fn drop(&mut self) {
        self.tx = None;
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
