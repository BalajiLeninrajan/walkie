//! walkie: push-to-talk dictation for Wayland.

#![allow(dead_code)] // wired up by the session in a later commit

mod config;
mod error;
mod history;
mod machine;
mod text;

/// Milliseconds since the Unix epoch.
#[must_use]
pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

fn main() {}
