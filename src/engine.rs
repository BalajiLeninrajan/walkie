//! transcribe.cpp, through the wrapper in `csrc/engine.c`.

use std::ffi::{CStr, CString, c_char, c_int};
use std::path::Path;
use std::ptr::{self, NonNull};

use crate::error::{Error, Result};

#[repr(C)]
struct RawEngine {
    _private: [u8; 0],
}

unsafe extern "C" {
    fn walkie_engine_open(
        model: *const c_char,
        threads: c_int,
        out: *mut *mut RawEngine,
        err: *mut *const c_char,
    ) -> c_int;
    fn walkie_engine_run(
        e: *mut RawEngine,
        pcm: *const f32,
        n: c_int,
        text: *mut *const c_char,
        err: *mut *const c_char,
    ) -> c_int;
    fn walkie_engine_close(e: *mut RawEngine);
}

/// A loaded model.
pub struct Engine(NonNull<RawEngine>);

// SAFETY: a transcribe.cpp session isn't tied to the thread that opened it;
// it just must not be used from two threads at once, which `&mut self` on
// `transcribe` guarantees.
unsafe impl Send for Engine {}

/// Copies a static status string from transcribe.cpp.
fn status_text(err: *const c_char) -> String {
    if err.is_null() {
        return "unknown error".into();
    }
    // SAFETY: transcribe_status_string returns NUL-terminated static strings.
    unsafe { CStr::from_ptr(err) }.to_string_lossy().into_owned()
}

impl Engine {
    pub fn open(model: &Path, threads: u32) -> Result<Self> {
        let path = CString::new(model.as_os_str().as_encoded_bytes())
            .map_err(|_| Error::Engine(format!("bad model path {}", model.display())))?;
        let (mut raw, mut err) = (ptr::null_mut(), ptr::null());
        let threads = c_int::try_from(threads).unwrap_or(c_int::MAX);
        // SAFETY: valid C string and out-pointers.
        if unsafe { walkie_engine_open(path.as_ptr(), threads, &raw mut raw, &raw mut err) } != 0 {
            return Err(Error::Engine(format!("loading {}: {}", model.display(), status_text(err))));
        }
        NonNull::new(raw).map(Self).ok_or_else(|| Error::Engine("open returned no session".into()))
    }

    /// Transcribes 16 kHz mono audio.
    pub fn transcribe(&mut self, pcm: &[f32]) -> Result<String> {
        let n = c_int::try_from(pcm.len()).map_err(|_| Error::Engine("recording too long".into()))?;
        let (mut text, mut err) = (ptr::null(), ptr::null());
        // SAFETY: valid session and buffer; `text` is copied before the next call.
        if unsafe { walkie_engine_run(self.0.as_ptr(), pcm.as_ptr(), n, &raw mut text, &raw mut err) } != 0 {
            return Err(Error::Engine(status_text(err)));
        }
        if text.is_null() {
            return Ok(String::new());
        }
        // SAFETY: transcribe_full_text returns a NUL-terminated string owned by the session.
        Ok(unsafe { CStr::from_ptr(text) }.to_string_lossy().into_owned())
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        // SAFETY: opened by walkie_engine_open, closed once.
        unsafe { walkie_engine_close(self.0.as_ptr()) };
    }
}
