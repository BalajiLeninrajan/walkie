//! walkie: push-to-talk dictation for Wayland.

mod audio;
mod config;
mod engine;
mod error;
mod history;
mod indicator;
mod ipc;
mod keyboard;
mod machine;
mod notify;
mod output;
mod session;
mod session_env;
mod sound;
mod text;

use std::process::ExitCode;

use config::{Config, Dirs};
use error::Result;
use ipc::Msg;

const USAGE: &str = "\
walkie: push-to-talk dictation for Wayland. Nothing stays in memory between dictations.

Usage: walkie <command>

  start     on key press: beep, start recording, start loading the model
  stop      on key release: stop recording, transcribe, type into the focused window
  cancel    drop the current recording without typing anything
  last      print the last transcript
  repaste   type the last transcript again
  status    print the indicator state record

A tap shorter than LOCK_MS locks the recording; the next press-and-release ends it.
Settings: ~/.config/walkie/config (see the README).";

/// Milliseconds since the Unix epoch.
#[must_use]
pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

fn main() -> ExitCode {
    // SAFETY: first thing in main, before any thread exists.
    unsafe { session_env::fill() };
    let dirs = Dirs::from_env();
    let cmd = std::env::args().nth(1).unwrap_or_default();
    match dispatch(&cmd, &dirs) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("walkie: {e}");
            if cmd == "start" || cmd == "repaste" {
                // kanata swallows stderr; make sure the user sees it.
                notify::Notifier::new(true).show(format!("walkie: {e}"), 5000);
            }
            ExitCode::FAILURE
        }
    }
}

fn dispatch(cmd: &str, dirs: &Dirs) -> Result<ExitCode> {
    match cmd {
        "start" => {
            let (cfg, warnings) = Config::load(dirs)?;
            session::start(&cfg, dirs, &warnings)?;
        }
        "stop" => {
            ipc::send(&dirs.run, Msg::Release)?;
        }
        "cancel" => {
            ipc::send(&dirs.run, Msg::Cancel)?;
        }
        "last" => match history::last(&dirs.state) {
            Some(t) => println!("{t}"),
            None => return Ok(ExitCode::FAILURE),
        },
        "repaste" => {
            let (cfg, _) = Config::load(dirs)?;
            if let Some(t) = history::last(&dirs.state) {
                output::deliver(&cfg, &format!("{t}{}", cfg.trailing))?;
            }
        }
        "status" => {
            let rec = std::fs::read_to_string(dirs.run.join("state")).unwrap_or_default();
            println!("{}", rec.trim_end());
        }
        "-V" | "--version" | "version" => println!("walkie {}", env!("CARGO_PKG_VERSION")),
        "-h" | "--help" | "help" => println!("{USAGE}"),
        _ => {
            eprintln!("{USAGE}");
            return Ok(ExitCode::from(2));
        }
    }
    Ok(ExitCode::SUCCESS)
}
