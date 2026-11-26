//! The control socket, `$XDG_RUNTIME_DIR/walkie/sock`.
//!
//! A running session listens on it; `walkie start/stop/cancel` connect and
//! send one line each: `press <ms>`, `release <ms>` or `cancel`, where `<ms>`
//! is the sender's monotonic clock when kanata ran it.
//!
//! `start` and `stop` both hold `run/lock` while they look for a session, so
//! a release can't slip in between `start` checking for a session and
//! binding the socket. A release that finds no session is left in
//! `run/release-pending` for a `start` that is about to bind.

use std::fs::{self, File};
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use crate::error::{IoContext, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Msg {
    Press(u64),
    Release(u64),
    Cancel,
}

impl Msg {
    fn encode(self) -> String {
        match self {
            Self::Press(t) => format!("press {t}\n"),
            Self::Release(t) => format!("release {t}\n"),
            Self::Cancel => "cancel\n".into(),
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let mut words = s.split_whitespace();
        let kind = words.next()?;
        let stamp = words.next().and_then(|t| t.parse().ok());
        match kind {
            "press" => Some(Self::Press(stamp?)),
            "release" => Some(Self::Release(stamp?)),
            "cancel" => Some(Self::Cancel),
            _ => None,
        }
    }
}

/// Milliseconds on the monotonic clock, comparable across processes.
pub fn mono_ms() -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: valid out-pointer; CLOCK_MONOTONIC always exists on Linux.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &raw mut ts) };
    ts.tv_sec as u64 * 1000 + ts.tv_nsec as u64 / 1_000_000
}

pub fn socket_path(run_dir: &Path) -> PathBuf {
    run_dir.join("sock")
}

fn pending_path(run_dir: &Path) -> PathBuf {
    run_dir.join("release-pending")
}

/// `flock` on `run/lock`, released on drop (or when every process holding
/// the open file has closed it).
pub struct Lock {
    _file: File,
}

impl Lock {
    pub fn acquire(run_dir: &Path) -> Result<Self> {
        fs::create_dir_all(run_dir).ctx(|| format!("creating {}", run_dir.display()))?;
        let path = run_dir.join("lock");
        let f = File::create(&path).ctx(|| format!("opening {}", path.display()))?;
        // SAFETY: valid fd.
        if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(std::io::Error::last_os_error()).ctx(|| format!("locking {}", path.display()));
        }
        Ok(Self { _file: f })
    }
}

/// Sends `msg` to the running session. `Ok(false)` if there is none.
pub fn send(run_dir: &Path, msg: Msg) -> Result<bool> {
    let path = socket_path(run_dir);
    match UnixStream::connect(&path) {
        Ok(mut s) => {
            s.write_all(msg.encode().as_bytes()).ctx(|| format!("writing to {}", path.display()))?;
            Ok(true)
        }
        Err(e) if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::ConnectionRefused) => Ok(false),
        Err(e) => Err(e).ctx(|| format!("connecting to {}", path.display())),
    }
}

/// True if a live session is listening (a stale socket file doesn't count).
pub fn session_alive(run_dir: &Path) -> bool {
    UnixStream::connect(socket_path(run_dir)).is_ok()
}

/// Handles `walkie stop`: tells the session, or leaves the release for a
/// `start` that hasn't bound its socket yet.
pub fn release(run_dir: &Path, at_ms: u64) -> Result<()> {
    let _lock = Lock::acquire(run_dir)?;
    if !send(run_dir, Msg::Release(at_ms))? {
        let path = pending_path(run_dir);
        fs::write(&path, at_ms.to_string()).ctx(|| format!("writing {}", path.display()))?;
    }
    Ok(())
}

/// Takes a release left by `walkie stop` that belongs to a press at
/// `pressed_ms` or later. Older ones are for an earlier session and dropped.
pub fn take_pending_release(run_dir: &Path, pressed_ms: u64) -> Option<u64> {
    let path = pending_path(run_dir);
    let stamp = fs::read_to_string(&path).ok()?.trim().parse().ok();
    let _ = fs::remove_file(&path);
    stamp.filter(|&t| t >= pressed_ms)
}

/// Binds the socket, replacing a stale one left by a crashed session. The
/// caller holds the lock, so no live session can own it.
pub fn bind(run_dir: &Path) -> Result<UnixListener> {
    let path = socket_path(run_dir);
    let _ = fs::remove_file(&path);
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

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("walkie-ipc-{name}-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn roundtrip_over_a_socket() {
        let dir = tmp("rt");
        assert!(!send(&dir, Msg::Press(1)).unwrap(), "no session yet");
        assert!(!session_alive(&dir));
        let listener = bind(&dir).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || serve(&listener, &tx));
        for m in [Msg::Press(5), Msg::Release(9), Msg::Cancel] {
            assert!(send(&dir, m).unwrap());
            assert_eq!(rx.recv().unwrap(), m);
        }
        assert!(session_alive(&dir));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn parse() {
        assert_eq!(Msg::parse("release 42\n"), Some(Msg::Release(42)));
        assert_eq!(Msg::parse("press"), None);
        assert_eq!(Msg::parse("cancel"), Some(Msg::Cancel));
        assert_eq!(Msg::parse("reboot 1"), None);
    }

    #[test]
    fn release_without_a_session_is_kept_for_the_next_start() {
        let dir = tmp("pending");
        release(&dir, 500).unwrap();
        assert_eq!(take_pending_release(&dir, 400), Some(500));
        assert_eq!(take_pending_release(&dir, 400), None, "taken once");
        release(&dir, 300).unwrap();
        assert_eq!(take_pending_release(&dir, 400), None, "older than the press");
        fs::remove_dir_all(dir).unwrap();
    }
}
