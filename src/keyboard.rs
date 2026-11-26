//! Typing through the Wayland virtual-keyboard protocol, as `wtype` does.
//!
//! The protocol has no "type this string" request: we upload a keymap in
//! which every character we need gets its own keycode, then press and
//! release those keycodes. Any Unicode character works, since its keysym is
//! just `0x0100_0000 + codepoint`.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs::File;
use std::io::Write;
use std::os::fd::{AsFd, FromRawFd};
use std::time::Instant;

use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_registry::WlRegistry, wl_seat::WlSeat};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, delegate_noop};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1, zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};

use crate::error::{Error, Result};

const KEYSYM_SHIFT_L: u32 = 0xffe1;
const KEYSYM_CONTROL_L: u32 = 0xffe3;
const MOD_SHIFT: u32 = 1;
const MOD_CONTROL: u32 = 4;

/// The keysym for a character, or `None` for control characters we skip.
pub fn keysym(c: char) -> Option<u32> {
    let cp = u32::from(c);
    match c {
        '\n' | '\r' => Some(0xff0d), // Return
        '\t' => Some(0xff09),        // Tab
        _ if c.is_control() => None,
        // Latin-1 keysyms equal their code points.
        ' '..='~' | '\u{a0}'..='\u{ff}' => Some(cp),
        _ => Some(0x0100_0000 + cp),
    }
}

/// A keymap with one keycode per keysym, and the keycode of each keysym.
#[derive(Debug)]
pub struct Keymap {
    pub text: String,
    pub codes: HashMap<u32, u32>,
}

impl Keymap {
    /// Builds a keymap holding `keysyms`, in first-seen order. Keycodes are
    /// evdev codes starting at 1 (XKB adds 8).
    pub fn new(keysyms: impl IntoIterator<Item = u32>) -> Self {
        let mut codes = HashMap::new();
        let mut order = Vec::new();
        for sym in keysyms {
            codes.entry(sym).or_insert_with(|| {
                order.push(sym);
                order.len() as u32
            });
        }
        let mut text = String::from("xkb_keymap {\nxkb_keycodes \"(unnamed)\" {\nminimum = 8;\n");
        let _ = writeln!(text, "maximum = {};", order.len() + 8 + 1);
        for i in 1..=order.len() {
            let _ = writeln!(text, "<K{i}> = {};", i + 8);
        }
        text += "};\nxkb_types \"(unnamed)\" { include \"complete\" };\n";
        text += "xkb_compatibility \"(unnamed)\" { include \"complete\" };\n";
        text += "xkb_symbols \"(unnamed)\" {\n";
        for (i, sym) in order.iter().enumerate() {
            let _ = writeln!(text, "key <K{}> {{[0x{sym:x}]}};", i + 1);
        }
        text += "};\n};\n";
        Self { text, codes }
    }
}

struct State;

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as wayland_client::Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(State: ignore WlSeat);
delegate_noop!(State: ZwpVirtualKeyboardManagerV1);
delegate_noop!(State: ZwpVirtualKeyboardV1);

/// A virtual keyboard on the default seat.
pub struct VirtualKeyboard {
    queue: EventQueue<State>,
    kb: ZwpVirtualKeyboardV1,
    epoch: Instant,
}

fn wl_err(e: impl std::fmt::Display) -> Error {
    Error::Wayland(e.to_string())
}

impl VirtualKeyboard {
    pub fn connect() -> Result<Self> {
        let conn = Connection::connect_to_env().map_err(wl_err)?;
        let (globals, queue) = registry_queue_init::<State>(&conn).map_err(wl_err)?;
        let qh = queue.handle();
        let seat: WlSeat = globals.bind(&qh, 1..=1, ()).map_err(|e| wl_err(format!("wl_seat: {e}")))?;
        let manager: ZwpVirtualKeyboardManagerV1 = globals
            .bind(&qh, 1..=1, ())
            .map_err(|e| wl_err(format!("the compositor has no virtual keyboard support: {e}")))?;
        let kb = manager.create_virtual_keyboard(&seat, &qh, ());
        Ok(Self { queue, kb, epoch: Instant::now() })
    }

    fn upload(&mut self, keymap: &Keymap) -> Result<()> {
        // SAFETY: memfd_create with a static NUL-terminated name.
        let fd = unsafe { libc::memfd_create(c"walkie-keymap".as_ptr(), libc::MFD_CLOEXEC) };
        if fd < 0 {
            return Err(wl_err(format!("memfd_create: {}", std::io::Error::last_os_error())));
        }
        // SAFETY: fd is a fresh descriptor we own.
        let mut file = unsafe { File::from_raw_fd(fd) };
        file.write_all(keymap.text.as_bytes()).and_then(|()| file.write_all(&[0])).map_err(wl_err)?;
        let size = keymap.text.len() as u32 + 1;
        self.kb.keymap(1 /* XKB_V1 */, file.as_fd(), size);
        self.roundtrip()
    }

    fn roundtrip(&mut self) -> Result<()> {
        self.queue.roundtrip(&mut State).map(drop).map_err(wl_err)
    }

    fn now(&self) -> u32 {
        self.epoch.elapsed().as_millis() as u32
    }

    fn tap(&mut self, code: u32) -> Result<()> {
        let t = self.now();
        self.kb.key(t, code, 1);
        self.kb.key(t, code, 0);
        // One roundtrip per key keeps the compositor's queue short; some
        // clients drop keys that arrive in a single burst.
        self.roundtrip()
    }

    /// Types `text` into the focused window.
    pub fn type_text(&mut self, text: &str) -> Result<()> {
        let syms: Vec<u32> = text.chars().filter_map(keysym).collect();
        if syms.is_empty() {
            return Ok(());
        }
        let keymap = Keymap::new(syms.iter().copied());
        self.upload(&keymap)?;
        for sym in syms {
            self.tap(keymap.codes[&sym])?;
        }
        Ok(())
    }

    /// Presses Ctrl+V, or Ctrl+Shift+V with `shift`.
    pub fn paste_shortcut(&mut self, shift: bool) -> Result<()> {
        let v = u32::from('v');
        let keymap = Keymap::new([v, KEYSYM_CONTROL_L, KEYSYM_SHIFT_L]);
        self.upload(&keymap)?;
        let mods = MOD_CONTROL | if shift { MOD_SHIFT } else { 0 };
        self.kb.modifiers(mods, 0, 0, 0);
        self.tap(keymap.codes[&v])?;
        self.kb.modifiers(0, 0, 0, 0);
        self.roundtrip()
    }
}

impl Drop for VirtualKeyboard {
    fn drop(&mut self) {
        self.kb.destroy();
        let _ = self.roundtrip();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keysyms() {
        assert_eq!(keysym('a'), Some(0x61));
        assert_eq!(keysym(' '), Some(0x20));
        assert_eq!(keysym('é'), Some(0xe9));
        assert_eq!(keysym('—'), Some(0x0100_2014));
        assert_eq!(keysym('😀'), Some(0x0101_f600));
        assert_eq!(keysym('\n'), Some(0xff0d));
        assert_eq!(keysym('\u{7}'), None);
    }

    #[test]
    fn keymap_dedups_and_numbers_from_one() {
        let km = Keymap::new("hello".chars().filter_map(keysym));
        assert_eq!(km.codes.len(), 4);
        assert_eq!(km.codes[&u32::from('h')], 1);
        assert_eq!(km.codes[&u32::from('l')], 3);
        assert_eq!(km.codes[&u32::from('o')], 4);
        assert!(km.text.contains("<K4> = 12;"));
        assert!(km.text.contains("key <K3> {[0x6c]};"));
        assert!(km.text.contains("maximum = 13;"));
    }
}
