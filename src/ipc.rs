//! The control socket, `$XDG_RUNTIME_DIR/walkie/sock`.
//!
//! A running session listens on it; `walkie start/stop/cancel` connect and
//! send one line each: `press`, `release` or `cancel`.

use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use crate::error::{IoContext, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Msg {
    Press,
    Release,
    Cancel,
}

impl Msg {
    fn as_str(self) -> &'static str {
        match self {
            Self::Press => "press",
            Self::Release => "release",
            Self::Cancel => "cancel",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "press" => Some(Self::Press),
            "release" => Some(Self::Release),
            "cancel" => Some(Self::Cancel),
            _ => None,
        }
    }
}

pub fn socket_path(run_dir: &Path) -> PathBuf {
    run_dir.join("sock")
}

/// Sends `msg` to the running session. `Ok(false)` if there is none.
pub fn send(run_dir: &Path, msg: Msg) -> Result<bool> {
    let path = socket_path(run_dir);
    match UnixStream::connect(&path) {
        Ok(mut s) => {
            s.write_all(format!("{}\n", msg.as_str()).as_bytes()).ctx(|| format!("writing to {}", path.display()))?;
            Ok(true)
        }
        Err(e) if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::ConnectionRefused) => Ok(false),
        Err(e) => Err(e).ctx(|| format!("connecting to {}", path.display())),
    }
}

/// Binds the socket, replacing a stale one left by a crashed session. The
/// caller holds the start lock, so no live session can own it.
pub fn bind(run_dir: &Path) -> Result<UnixListener> {
    let path = socket_path(run_dir);
    let _ = std::fs::remove_file(&path);
    UnixListener::bind(&path).ctx(|| format!("binding {}", path.display()))
}

/// Forwards every message that arrives on `listener` to `tx`, until the
/// receiver goes away. Runs on its own thread.
pub fn serve(listener: &UnixListener, tx: &Sender<Msg>) {
    for conn in listener.incoming() {
        let Ok(conn) = conn else { continue };
        for line in BufReader::new(conn).lines() {
            let Ok(line) = line else { break };
            if let Some(msg) = Msg::parse(&line) {
                if tx.send(msg).is_err() {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_over_a_socket() {
        let dir = std::env::temp_dir().join(format!("walkie-ipc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!send(&dir, Msg::Press).unwrap(), "no session yet");
        let listener = bind(&dir).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || serve(&listener, &tx));
        for m in [Msg::Press, Msg::Release, Msg::Cancel] {
            assert!(send(&dir, m).unwrap());
            assert_eq!(rx.recv().unwrap(), m);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn parse_ignores_junk() {
        assert_eq!(Msg::parse("release\n"), Some(Msg::Release));
        assert_eq!(Msg::parse("reboot"), None);
    }
}
