//! One dictation, from key press to typed text.
//!
//! `walkie start` binds the control socket, forks, and returns at once. The
//! child is the session: it records, loads the model on another thread
//! while the user talks, waits for the release, transcribes, types, and
//! exits. Nothing stays resident between dictations.

use std::fs::{self, File};
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::audio::{Beeper, PipeWire, Recorder};
use crate::config::{Config, Dirs};
use crate::engine::Engine;
use crate::error::{Error, IoContext, Result};
use crate::indicator::{Indicator, Phase, Status};
use crate::ipc::{self, Msg};
use crate::machine::{Action, KeyMachine};
use crate::notify::Notifier;
use crate::sound::Beep;
use crate::{history, output, text};

/// Under 0.1 s of audio there is nothing to transcribe.
const MIN_SAMPLES: usize = 1600;

/// Extra time past `MAX_SECONDS` before the watchdog gives up on a session
/// stuck transcribing or typing.
const WATCHDOG_GRACE: Duration = Duration::from_secs(120);

/// How long a `POSTPROC` filter may run.
const POSTPROC_TIMEOUT: Duration = Duration::from_secs(10);

/// What the session knows when it starts.
struct Started<'a> {
    cfg: &'a Config,
    dirs: &'a Dirs,
    /// Monotonic ms when kanata ran `walkie start`.
    pressed_ms: u64,
    /// A `walkie stop` that arrived before the socket existed.
    pending_release: Option<u64>,
}

/// Handles `walkie start`. `pressed_ms` is the monotonic clock at entry.
pub fn start(cfg: &Config, dirs: &Dirs, warnings: &[String], pressed_ms: u64) -> Result<()> {
    fs::create_dir_all(&dirs.run).ctx(|| format!("creating {}", dirs.run.display()))?;
    fs::set_permissions(&dirs.run, fs::Permissions::from_mode(0o700))
        .ctx(|| format!("securing {}", dirs.run.display()))?;

    // Serializes with other starts and with stops (see ipc).
    let lock = ipc::Lock::acquire(&dirs.run)?;
    if ipc::send(&dirs.run, Msg::Press(pressed_ms))? {
        return Ok(()); // key repeat, or the press that ends a locked recording
    }
    let listener = ipc::bind(&dirs.run)?;
    let pending_release = ipc::take_pending_release(&dirs.run, pressed_ms);

    // SAFETY: walkie is single-threaded until here, so the child gets a
    // consistent copy of the process.
    match unsafe { libc::fork() } {
        -1 => Err(std::io::Error::last_os_error()).ctx(|| "fork".into()),
        0 => {
            drop(lock); // the parent releases it when it exits
            detach(dirs);
            let started = Started { cfg, dirs, pressed_ms, pending_release };
            let code = i32::from(run(&started, &listener, warnings).is_err());
            // _exit, not exit: skip C++ static destructors, which could run
            // under a model loader thread that a cancel left behind.
            // SAFETY: ends the process; nothing after this runs.
            unsafe { libc::_exit(code) }
        }
        _ => Ok(()),
    }
}

/// Starts a new session, and points stdin/stdout at /dev/null and stderr
/// at `run/log` so the key handler that started us isn't held open.
fn detach(dirs: &Dirs) {
    // Truncate, then append: a session still transcribing keeps writing
    // whole lines to the same file instead of leaving a hole in it.
    let path = dirs.run.join("log");
    let log = fs::write(&path, "")
        .and_then(|()| fs::OpenOptions::new().append(true).open(&path))
        .or_else(|_| File::create("/dev/null"));
    let null = File::open("/dev/null");
    // SAFETY: setsid/dup2 on fds we own; failures leave the defaults in place.
    unsafe {
        libc::setsid();
        if let Ok(null) = &null {
            libc::dup2(null.as_raw_fd(), 0);
            libc::dup2(null.as_raw_fd(), 1);
        }
        if let Ok(log) = &log {
            libc::dup2(log.as_raw_fd(), 2);
        }
    }
}

/// Feedback channels that outlive a failure, so errors can be reported.
struct Feedback {
    beeper: Beeper,
    notifier: Notifier,
}

impl Feedback {
    fn fail(&self, e: &Error) {
        eprintln!("walkie: {e}");
        self.beeper.play(Beep::Error);
        self.notifier.show(format!("walkie: {e}"), 5000);
    }
}

fn run(st: &Started, listener: &UnixListener, warnings: &[String]) -> Result<()> {
    let (cfg, dirs) = (st.cfg, st.dirs);
    for w in warnings {
        eprintln!("walkie: config {w}");
    }
    start_watchdog(&dirs.run, Duration::from_secs(cfg.max_seconds) + WATCHDOG_GRACE);
    let status = Arc::new(Status::default());
    status.set_phase(Phase::Recording);
    let test_wav = std::env::var_os("WALKIE_TEST_WAV");

    // Open the microphone before anything else so the first word isn't cut.
    let pw: Result<Option<Arc<PipeWire>>> =
        if test_wav.is_none() || cfg.sounds { PipeWire::connect().map(|p| Some(Arc::new(p))) } else { Ok(None) };
    let beeps = if cfg.sounds { pw.as_ref().ok().cloned().flatten() } else { None };
    let feedback =
        Feedback { beeper: Beeper::new(beeps, &cfg.sound_dir, cfg.sound_volume), notifier: Notifier::new(cfg.notify) };
    let result = (|| {
        let recorder = match (&test_wav, pw?) {
            (Some(wav), _) => Recorder::from_wav(wav.as_ref(), Arc::clone(&status))?,
            (None, Some(pw)) => Recorder::pipewire(pw, Arc::clone(&status))?,
            (None, None) => unreachable!("PipeWire is always opened without a test file"),
        };
        let indicator = Indicator::start(&dirs.run, Arc::clone(&status))?;
        record(st, listener, recorder, &indicator, &feedback)
    })();
    // Don't leave a dead socket for the next `start` to trip over. After the
    // release the path may already belong to a newer session.
    if !status.released() {
        let _ = fs::remove_file(ipc::socket_path(&dirs.run));
    }
    if let Err(e) = &result {
        feedback.fail(e);
    }
    feedback.beeper.drain();
    result
}

/// Exits the session if it is still running at `after`, leaving the
/// indicator idle. Covers a hung typing step or model.
fn start_watchdog(run_dir: &std::path::Path, after: Duration) {
    let run_dir = run_dir.to_path_buf();
    thread::spawn(move || {
        thread::sleep(after);
        eprintln!("walkie: session still running after {} s, giving up", after.as_secs());
        crate::indicator::force_idle(&run_dir);
        // SAFETY: ends the process.
        unsafe { libc::_exit(3) }
    });
}

/// Waits while a newer session is recording: its key may be held down, and
/// typing then would turn our text into shortcuts. Its release (or lock)
/// removes or keeps its socket; we go once it's gone or the deadline hits.
fn wait_for_newer_session(run_dir: &std::path::Path, deadline: Instant) {
    let t = Instant::now();
    while ipc::session_alive(run_dir) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(30));
    }
    if t.elapsed() > Duration::from_millis(30) {
        eprintln!("walkie: waited {} ms for a newer session's key release", t.elapsed().as_millis());
    }
}

fn record(
    st: &Started,
    listener: &UnixListener,
    recorder: Recorder,
    indicator: &Indicator,
    fb: &Feedback,
) -> Result<()> {
    let (cfg, dirs) = (st.cfg, st.dirs);
    let t_start = Instant::now();
    let loader: JoinHandle<(Result<Engine>, Duration)> = {
        let (model, threads) = (cfg.model.clone(), cfg.threads);
        thread::spawn(move || {
            let t = Instant::now();
            (Engine::open(&model, threads), t.elapsed())
        })
    };
    fb.beeper.play(Beep::Start);
    fb.notifier.show("● Recording", 0);

    let (tx, rx) = mpsc::channel();
    let listener = listener.try_clone().ctx(|| "cloning the control socket".into())?;
    thread::spawn(move || ipc::serve(&listener, &tx));

    let mut keys = KeyMachine::new(st.pressed_ms, cfg.lock_ms);
    let deadline = t_start + Duration::from_secs(cfg.max_seconds);
    let mut pending = st.pending_release.map(Msg::Release);
    loop {
        let msg = match pending.take() {
            Some(m) => Ok(m),
            None => rx.recv_timeout(deadline.saturating_duration_since(Instant::now())),
        };
        let action = match msg {
            Ok(Msg::Press(t)) => keys.press(t),
            Ok(Msg::Release(t)) => keys.release(t),
            Ok(Msg::Cancel) => {
                drop(recorder);
                fb.beeper.play(Beep::Error);
                fb.notifier.show("Cancelled", 800);
                return Ok(());
            }
            Err(RecvTimeoutError::Timeout) => {
                eprintln!("walkie: reached MAX_SECONDS={}, finishing", cfg.max_seconds);
                Action::Finish
            }
            Err(RecvTimeoutError::Disconnected) => Action::Finish,
        };
        match action {
            Action::None => {}
            Action::Lock => {
                indicator.status().set_locked(true);
                fb.beeper.play(Beep::Lock);
                fb.notifier.show("● Recording (locked). Press again to finish.", 0);
            }
            Action::Finish => break,
        }
    }
    let released = Instant::now();
    // A new press from here on starts a new session.
    {
        let _lock = ipc::Lock::acquire(&dirs.run)?;
        let _ = fs::remove_file(ipc::socket_path(&dirs.run));
        indicator.status().set_released();
    }

    thread::sleep(Duration::from_millis(cfg.tail_ms));
    let first_audio_ms = recorder.first_audio_ms();
    let mic_error = recorder.error();
    let pcm = recorder.stop();
    if first_audio_ms.is_none() && std::env::var_os("WALKIE_TEST_WAV").is_none() {
        return Err(Error::Audio(
            mic_error.unwrap_or_else(|| "no audio arrived from the microphone; is a default source set?".into()),
        ));
    }
    indicator.status().set_phase(Phase::Transcribing);
    fb.beeper.play(Beep::Stop);
    fb.notifier.show("… Transcribing", 0);

    let (engine, load_time) = loader.join().map_err(|_| Error::Engine("model loader panicked".into()))?;
    let mut engine = engine?;
    let raw = if pcm.len() < MIN_SAMPLES { String::new() } else { engine.transcribe(&pcm)? };
    drop(engine);

    let mut out = text::normalize(&raw);
    if cfg.digits && !out.is_empty() {
        out = text::digits(&out);
    }
    if !cfg.postproc.is_empty() && !out.is_empty() {
        let (filtered, err) = text::postproc(&cfg.postproc, &out, POSTPROC_TIMEOUT);
        if let Some(err) = err {
            eprintln!("walkie: {err}");
        }
        out = filtered;
    }
    eprintln!(
        "walkie: press_to_audio_ms={} load_ms={} audio_s={:.2} release_to_text_ms={} {}",
        first_audio_ms.map_or_else(|| "none".into(), |t| t.saturating_sub(st.pressed_ms).to_string()),
        load_time.as_millis(),
        pcm.len() as f64 / f64::from(crate::audio::RATE),
        released.elapsed().as_millis(),
        resource_usage()
    );
    if out.is_empty() {
        fb.notifier.show("(no speech)", 1000);
        return Ok(());
    }
    if let Err(e) = history::append(&dirs.state, &out) {
        eprintln!("walkie: {e}");
    }
    wait_for_newer_session(&dirs.run, t_start + Duration::from_secs(cfg.max_seconds) + WATCHDOG_GRACE / 2);
    output::deliver(cfg, &format!("{out}{}", cfg.trailing))?;
    fb.notifier.show(out, 1500);
    Ok(())
}

/// Peak RSS and CPU time so far, for the log line the benchmarks read.
fn resource_usage() -> String {
    let hwm = fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| s.lines().find_map(|l| l.strip_prefix("VmHWM:").map(|v| v.trim().to_owned())))
        .unwrap_or_default();
    // SAFETY: getrusage fills the zeroed struct we pass.
    let ru = unsafe {
        let mut ru: libc::rusage = std::mem::zeroed();
        libc::getrusage(libc::RUSAGE_SELF, &raw mut ru);
        ru
    };
    let ms = |t: libc::timeval| t.tv_sec * 1000 + t.tv_usec / 1000;
    format!("peak_rss={} cpu_ms={}", hwm.replace(' ', ""), ms(ru.ru_utime) + ms(ru.ru_stime))
}
