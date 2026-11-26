//! walkie: push-to-talk dictation for Wayland.

#![allow(dead_code)] // wired up by the session in a later commit

mod audio;
mod config;
mod engine;
mod error;
mod history;
mod indicator;
mod machine;
mod sound;
mod text;

/// Milliseconds since the Unix epoch.
#[must_use]
pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

fn main() {}
