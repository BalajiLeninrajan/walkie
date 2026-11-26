//! Microphone capture and beeps over PipeWire, through `csrc/pw.c`.

use std::ffi::{CStr, c_char, c_int, c_void};
use std::path::Path;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::error::{Error, Result};
use crate::indicator::Status;
use crate::sound::{self, Beep};

pub const RATE: u32 = 16_000;

#[repr(C)]
struct RawPw {
    _private: [u8; 0],
}

type PcmCallback = unsafe extern "C" fn(user: *mut c_void, samples: *const i16, n: u32);

unsafe extern "C" {
    fn walkie_pw_new(err: *mut c_char, errlen: usize) -> *mut RawPw;
    fn walkie_pw_error(pw: *mut RawPw) -> *const c_char;
    fn walkie_pw_capture_start(pw: *mut RawPw, cb: PcmCallback, user: *mut c_void) -> c_int;
    fn walkie_pw_capture_stop(pw: *mut RawPw);
    fn walkie_pw_play(pw: *mut RawPw, samples: *const i16, n: u32, rate: u32) -> c_int;
    fn walkie_pw_wait_playback(pw: *mut RawPw, timeout_ms: c_int);
    fn walkie_pw_free(pw: *mut RawPw);
}

/// A PipeWire connection with its own thread loop.
pub struct PipeWire(NonNull<RawPw>);

// SAFETY: every walkie_pw_* entry point takes the thread-loop lock, so the
// handle can be used from any thread.
unsafe impl Send for PipeWire {}
// SAFETY: as above; no method hands out references into the C state.
unsafe impl Sync for PipeWire {}

impl PipeWire {
    pub fn connect() -> Result<Self> {
        let mut err = [0 as c_char; 256];
        // SAFETY: err is a writable buffer of the given length.
        let raw = unsafe { walkie_pw_new(err.as_mut_ptr(), err.len()) };
        NonNull::new(raw).map(Self).ok_or_else(|| {
            // SAFETY: walkie_pw_new NUL-terminates err on failure.
            Error::Audio(unsafe { CStr::from_ptr(err.as_ptr()) }.to_string_lossy().into_owned())
        })
    }

    fn last_error(&self) -> Option<String> {
        // SAFETY: the handle is valid; the returned string lives as long as it.
        let p = unsafe { walkie_pw_error(self.0.as_ptr()) };
        // SAFETY: non-null results are NUL-terminated.
        (!p.is_null()).then(|| unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
    }

    pub fn play(&self, pcm: &sound::Pcm) {
        // SAFETY: the samples are copied before walkie_pw_play returns.
        unsafe { walkie_pw_play(self.0.as_ptr(), pcm.samples.as_ptr(), pcm.samples.len() as u32, pcm.rate) };
    }

    pub fn wait_playback(&self, timeout: Duration) {
        // SAFETY: valid handle.
        unsafe { walkie_pw_wait_playback(self.0.as_ptr(), timeout.as_millis().try_into().unwrap_or(c_int::MAX)) };
    }
}

impl Drop for PipeWire {
    fn drop(&mut self) {
        // SAFETY: the handle came from walkie_pw_new and is freed once.
        unsafe { walkie_pw_free(self.0.as_ptr()) };
    }
}

/// Samples collected so far, plus the level meter.
pub struct Sink {
    samples: Mutex<Vec<i16>>,
    status: Arc<Status>,
    /// Unix ms when the first samples arrived, 0 before that.
    first_ms: AtomicU64,
}

impl Sink {
    fn new(samples: Vec<i16>, status: Arc<Status>) -> Self {
        Self { samples: Mutex::new(samples), status, first_ms: AtomicU64::new(0) }
    }

    fn push(&self, chunk: &[i16]) {
        if chunk.is_empty() {
            return;
        }
        let sum: f64 = chunk.iter().map(|&s| f64::from(s) * f64::from(s)).sum();
        let _ = self.first_ms.compare_exchange(0, crate::now_ms(), Ordering::Relaxed, Ordering::Relaxed);
        let rms = (sum / chunk.len() as f64).sqrt() / 32768.0;
        self.status.report_rms(rms as f32);
        self.samples.lock().unwrap_or_else(std::sync::PoisonError::into_inner).extend_from_slice(chunk);
    }
}

unsafe extern "C" fn on_pcm(user: *mut c_void, samples: *const i16, n: u32) {
    // SAFETY: `user` is the Sink that Recorder keeps alive until capture has
    // stopped, and PipeWire hands us `n` valid samples.
    let (sink, chunk) = unsafe { (&*user.cast::<Sink>(), std::slice::from_raw_parts(samples, n as usize)) };
    sink.push(chunk);
}

enum Source {
    PipeWire(Arc<PipeWire>),
    /// Test input: a WAV file fed in real time.
    File {
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
    },
}

/// A running recording.
pub struct Recorder {
    sink: Arc<Sink>,
    source: Source,
}

impl Recorder {
    /// Starts recording from the default PipeWire source.
    pub fn pipewire(pw: Arc<PipeWire>, status: Arc<Status>) -> Result<Self> {
        let sink = Arc::new(Sink::new(Vec::with_capacity(RATE as usize * 30), status));
        let user = Arc::as_ptr(&sink).cast_mut().cast::<c_void>();
        // SAFETY: `user` stays valid while `self.sink` lives, and stop()/drop
        // end the capture before the Sink can be freed.
        if unsafe { walkie_pw_capture_start(pw.0.as_ptr(), on_pcm, user) } != 0 {
            return Err(Error::Audio(pw.last_error().unwrap_or_else(|| "could not start recording".into())));
        }
        Ok(Self { sink, source: Source::PipeWire(pw) })
    }

    /// Feeds a 16 kHz mono WAV in 100 ms chunks at real-time pace, then
    /// silence until stopped, like a microphone would. Used by the
    /// benchmarks; set `WALKIE_TEST_WAV`.
    pub fn from_wav(path: &Path, status: Arc<Status>) -> Result<Self> {
        let bytes = std::fs::read(path).map_err(|e| Error::Audio(format!("{}: {e}", path.display())))?;
        let pcm = sound::parse_wav(&bytes)
            .filter(|p| p.rate == RATE)
            .ok_or_else(|| Error::Audio(format!("{}: need a 16 kHz 16-bit WAV", path.display())))?;
        let sink = Arc::new(Sink::new(Vec::new(), status));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (sink, stop) = (Arc::clone(&sink), Arc::clone(&stop));
            std::thread::spawn(move || {
                let t0 = Instant::now();
                let silence = [0i16; RATE as usize / 10];
                let chunks = pcm.samples.chunks(RATE as usize / 10).chain(std::iter::repeat(&silence[..]));
                for (i, chunk) in chunks.enumerate() {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    sink.push(chunk);
                    let next = t0 + Duration::from_millis(100 * (i as u64 + 1));
                    std::thread::sleep(next.saturating_duration_since(Instant::now()));
                }
            })
        };
        Ok(Self { sink, source: Source::File { stop, thread: Some(thread) } })
    }

    /// Unix ms when the first audio arrived, if any has.
    pub fn first_audio_ms(&self) -> Option<u64> {
        Some(self.sink.first_ms.load(Ordering::Relaxed)).filter(|&t| t != 0)
    }

    /// Stops recording and returns the audio as floats in -1..1.
    pub fn stop(mut self) -> Vec<f32> {
        match &mut self.source {
            // SAFETY: valid handle; after this returns the callback can't run.
            Source::PipeWire(pw) => unsafe { walkie_pw_capture_stop(pw.0.as_ptr()) },
            Source::File { stop, thread } => {
                stop.store(true, Ordering::Relaxed);
                if let Some(t) = thread.take() {
                    let _ = t.join();
                }
            }
        }
        let samples = std::mem::take(&mut *self.sink.samples.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
        samples.iter().map(|&s| f32::from(s) / 32768.0).collect()
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        if let Source::PipeWire(pw) = &self.source {
            // SAFETY: idempotent; makes sure the callback is gone before the Sink.
            unsafe { walkie_pw_capture_stop(pw.0.as_ptr()) };
        }
    }
}

/// Plays the beeps, or nothing when sounds are off or PipeWire is missing.
pub struct Beeper {
    pw: Option<Arc<PipeWire>>,
    dir: std::path::PathBuf,
    volume: f32,
}

impl Beeper {
    pub fn new(pw: Option<Arc<PipeWire>>, dir: &Path, volume: f32) -> Self {
        Self { pw, dir: dir.to_path_buf(), volume }
    }

    pub fn play(&self, beep: Beep) {
        if let Some(pw) = &self.pw {
            pw.play(&beep.load(&self.dir, self.volume));
        }
    }

    /// Lets the last beep finish before the process exits.
    pub fn drain(&self) {
        if let Some(pw) = &self.pw {
            pw.wait_playback(Duration::from_millis(500));
        }
    }
}
