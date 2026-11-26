//! Getting the transcript into the focused window.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::config::{Config, TypeMode};
use crate::error::{Error, IoContext, Result};
use crate::keyboard::VirtualKeyboard;

/// Types or pastes `text` according to `TYPE_MODE`.
///
/// With `WALKIE_TEST_OUT=<file>` set, appends `<unix ms> <text>` to that
/// file instead; the end-to-end benchmark uses it to time key release to text.
pub fn deliver(cfg: &Config, text: &str) -> Result<()> {
    if let Some(path) = std::env::var_os("WALKIE_TEST_OUT") {
        let line = format!("{} {text}\n", crate::now_ms());
        return std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .and_then(|mut f| f.write_all(line.as_bytes()))
            .ctx(|| "writing WALKIE_TEST_OUT".into());
    }
    match cfg.type_mode {
        TypeMode::Type => VirtualKeyboard::connect()?.type_text(text),
        TypeMode::Paste => {
            copy(text)?;
            let terminal = focused_app_id().is_some_and(|app| cfg.terminals.contains(&app));
            VirtualKeyboard::connect()?.paste_shortcut(terminal)
        }
    }
}

/// Puts `text` on the clipboard with `wl-copy`.
///
/// Owning a Wayland selection means staying alive to serve it, which would
/// break "nothing resident". `wl-copy` already forks a small server for
/// exactly that, so we hand it over.
fn copy(text: &str) -> Result<()> {
    let mut child = Command::new("wl-copy")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .ctx(|| "running wl-copy (install wl-clipboard)".into())?;
    child.stdin.take().expect("piped stdin").write_all(text.as_bytes()).ctx(|| "writing to wl-copy".into())?;
    let status = child.wait().ctx(|| "waiting for wl-copy".into())?;
    if status.success() { Ok(()) } else { Err(Error::Wayland(format!("wl-copy exited with {status}"))) }
}

/// The focused window's `app_id`, asked from niri over its IPC socket.
fn focused_app_id() -> Option<String> {
    let sock = std::env::var_os("NIRI_SOCKET")?;
    let mut stream = UnixStream::connect(sock).ok()?;
    stream.set_read_timeout(Some(Duration::from_millis(500))).ok()?;
    stream.write_all(b"\"FocusedWindow\"\n").ok()?;
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply).ok()?;
    json_string_field(&reply, "app_id")
}

/// Pulls `"key":"value"` out of a JSON line without a JSON parser. Handles
/// the escapes niri emits for app ids (`\"` and `\\`).
fn json_string_field(json: &str, key: &str) -> Option<String> {
    let start = json.find(&format!("\"{key}\":\""))? + key.len() + 4;
    let mut out = String::new();
    let mut chars = json[start..].chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => out.push(chars.next()?),
            c => out.push(c),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_id_from_niri_reply() {
        let r = r#"{"Ok":{"FocusedWindow":{"id":7,"title":"x \"y\"","app_id":"foot","pid":1}}}"#;
        assert_eq!(json_string_field(r, "app_id").as_deref(), Some("foot"));
        assert_eq!(json_string_field(r, "title").as_deref(), Some("x \"y\""));
        assert_eq!(json_string_field(r#"{"Ok":{"FocusedWindow":null}}"#, "app_id"), None);
        assert_eq!(json_string_field(r#"{"app_id":null}"#, "app_id"), None);
    }
}
