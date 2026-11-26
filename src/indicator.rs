//! The state file the DMS plugin reads: `$XDG_RUNTIME_DIR/walkie/state`.
//!
//! Format (version 1), one line of `key=value` fields separated by spaces,
//! padded with spaces to exactly 63 bytes plus a newline:
//!
//! ```text
//! v=1 state=recording level=0.42 locked=0
//! ```
//!
//! - `v`: format version. Bumped when a field changes meaning or goes away;
//!   new fields may be added without a bump, so readers ignore unknown keys.
//! - `state`: `idle`, `recording` or `transcribing`.
//! - `level`: microphone level from 0.00 to 1.00 (-60 dBFS to 0 dBFS, peak
//!   RMS over the last update interval). 0.00 outside `recording`.
//! - `locked`: 1 if a tap locked the recording.
//!
//! Every update rewrites the whole record in place at offset 0 with a single
//! `pwrite`, so the file never shrinks, its inode never changes (inotify
//! watches stay valid) and a reader never sees an empty file. Updates come
//! every 40 ms while recording and every 250 ms otherwise while a session
//! runs; a reader can treat a non-idle state older than ~2 s as stale (the
//! session died). The file is left in place, as `idle`, between dictations.
//!
//! Sessions can overlap: a new press while the last one is still
//! transcribing starts a second session. The file then belongs to the newer
//! one; the older one stops writing as soon as it sees a live socket that
//! isn't its own.

use std::fs::{File, OpenOptions};
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::error::{IoContext, Result};

pub const VERSION: u32 = 1;
const RECORD_LEN: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Phase {
    Idle = 0,
    Recording = 1,
    Transcribing = 2,
}

impl Phase {
    fn name(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Recording => "recording",
            Self::Transcribing => "transcribing",
        }
    }

    fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Recording,
            2 => Self::Transcribing,
            _ => Self::Idle,
        }
    }
}

/// Formats one record. `level` is clamped to 0..=1.
pub fn format_record(phase: Phase, level: f32, locked: bool) -> [u8; RECORD_LEN] {
    let level = if phase == Phase::Recording { level.clamp(0.0, 1.0) } else { 0.0 };
    let line = format!("v={VERSION} state={} level={level:.2} locked={}", phase.name(), u8::from(locked));
    let mut rec = [b' '; RECORD_LEN];
    rec[..line.len()].copy_from_slice(line.as_bytes());
    rec[RECORD_LEN - 1] = b'\n';
    rec
}

/// Maps an RMS amplitude (0..1 full scale) to the 0..1 meter scale.
pub fn meter(rms: f32) -> f32 {
    if rms <= 1e-6 {
        return 0.0;
    }
    ((20.0 * rms.log10() + 60.0) / 60.0).clamp(0.0, 1.0)
}

/// Shared between the audio callback, the session and the writer thread.
#[derive(Debug, Default)]
pub struct Status {
    phase: AtomicU8,
    locked: AtomicBool,
    /// Peak RMS since the last write, as `f32` bits. Positive floats order
    /// the same as their bit patterns, so `fetch_max` works on the bits.
    peak_rms: AtomicU32,
    /// Set once this session has given up its socket (after the release).
    released: AtomicBool,
    stop: AtomicBool,
}

impl Status {
    pub fn set_phase(&self, p: Phase) {
        self.phase.store(p as u8, Ordering::Relaxed);
    }

    pub fn set_released(&self) {
        self.released.store(true, Ordering::Relaxed);
    }

    pub fn released(&self) -> bool {
        self.released.load(Ordering::Relaxed)
    }

    pub fn set_locked(&self, l: bool) {
        self.locked.store(l, Ordering::Relaxed);
    }

    /// Called from the audio thread with each chunk's RMS.
    pub fn report_rms(&self, rms: f32) {
        self.peak_rms.fetch_max(rms.max(0.0).to_bits(), Ordering::Relaxed);
    }

    fn take_rms(&self) -> f32 {
        f32::from_bits(self.peak_rms.swap(0, Ordering::Relaxed))
    }
}

/// Keeps the state file current from a background thread.
pub struct Indicator {
    file: Arc<File>,
    status: Arc<Status>,
    socket: PathBuf,
    thread: Option<JoinHandle<()>>,
}

/// True once a newer session owns the state file: this one has released its
/// socket and a socket exists again.
fn superseded(status: &Status, socket: &Path) -> bool {
    status.released() && socket.exists()
}

/// Writes `idle` without a running session. The session watchdog uses it.
pub fn force_idle(run_dir: &Path) {
    if let Ok(f) = OpenOptions::new().write(true).open(run_dir.join("state")) {
        let _ = f.write_at(&format_record(Phase::Idle, 0.0, false), 0);
    }
}

impl Indicator {
    pub fn start(run_dir: &Path, status: Arc<Status>) -> Result<Self> {
        let path = run_dir.join("state");
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .ctx(|| format!("opening {}", path.display()))?;
        file.set_len(RECORD_LEN as u64).ctx(|| format!("resizing {}", path.display()))?;
        let file = Arc::new(file);
        let socket = crate::ipc::socket_path(run_dir);
        let thread = {
            let (file, status, socket) = (Arc::clone(&file), Arc::clone(&status), socket.clone());
            std::thread::Builder::new()
                .name("indicator".into())
                .spawn(move || writer(&file, &status, &socket))
                .ctx(|| "starting the indicator thread".into())?
        };
        Ok(Self { file, status, socket, thread: Some(thread) })
    }

    pub fn status(&self) -> &Arc<Status> {
        &self.status
    }
}

impl Drop for Indicator {
    /// Stops the writer and leaves the file saying `idle`.
    fn drop(&mut self) {
        self.status.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        if !superseded(&self.status, &self.socket) {
            let _ = self.file.write_at(&format_record(Phase::Idle, 0.0, false), 0);
        }
    }
}

fn writer(file: &File, status: &Status, socket: &Path) {
    let mut since_write = Duration::MAX;
    let tick = Duration::from_millis(40);
    let mut last = [0u8; RECORD_LEN];
    while !status.stop.load(Ordering::Relaxed) {
        let phase = Phase::from_u8(status.phase.load(Ordering::Relaxed));
        let level = meter(status.take_rms());
        let rec = format_record(phase, level, status.locked.load(Ordering::Relaxed));
        // Write on every change, and at least every 250 ms as a heartbeat.
        let due = rec != last || since_write >= Duration::from_millis(250);
        if due && !superseded(status, socket) {
            let _ = file.write_at(&rec, 0);
            last = rec;
            since_write = Duration::ZERO;
        }
        std::thread::sleep(tick);
        since_write = since_write.saturating_add(tick);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(r: &[u8]) -> &str {
        std::str::from_utf8(r).unwrap()
    }

    #[test]
    fn record_is_fixed_width() {
        for (p, l, k) in [(Phase::Idle, 0.0, false), (Phase::Recording, 0.4, true), (Phase::Transcribing, 1.0, true)] {
            let r = format_record(p, l, k);
            assert_eq!(r.len(), 64);
            assert_eq!(r[63], b'\n');
            assert!(text(&r).starts_with("v=1 state="));
        }
    }

    #[test]
    fn record_fields() {
        assert_eq!(
            text(&format_record(Phase::Recording, 0.4242, true)).trim_end(),
            "v=1 state=recording level=0.42 locked=1"
        );
        // Level is only meaningful while recording, and is clamped.
        assert_eq!(
            text(&format_record(Phase::Transcribing, 0.9, false)).trim_end(),
            "v=1 state=transcribing level=0.00 locked=0"
        );
        assert!(text(&format_record(Phase::Recording, 7.0, false)).contains("level=1.00"));
    }

    #[test]
    fn meter_scale() {
        assert!(meter(0.0).abs() < f32::EPSILON);
        assert!(meter(0.001).abs() < 1e-3); // -60 dBFS
        assert!((meter(1.0) - 1.0).abs() < 1e-6);
        assert!((meter(0.031_622_8) - 0.5).abs() < 1e-3); // -30 dBFS
    }

    #[test]
    fn peak_rms_keeps_the_max() {
        let s = Status::default();
        s.report_rms(0.1);
        s.report_rms(0.3);
        s.report_rms(0.2);
        assert!((s.take_rms() - 0.3).abs() < f32::EPSILON);
        assert!(s.take_rms().abs() < f32::EPSILON);
    }
}
