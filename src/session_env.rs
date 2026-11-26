//! kanata often runs as a system service, outside the graphical session, so
//! its children lack the variables that point at the session's sockets.
//! Fill them in from the usual locations.

use std::env;
use std::path::Path;

/// Sets `XDG_RUNTIME_DIR`, `WAYLAND_DISPLAY`, `DBUS_SESSION_BUS_ADDRESS` and
/// `NIRI_SOCKET` when they are missing.
///
/// # Safety
///
/// Must run before any other thread exists (it calls `env::set_var`).
pub unsafe fn fill() {
    let set = |k: &str, v: &str| {
        // SAFETY: the caller guarantees we're still single-threaded.
        unsafe { env::set_var(k, v) };
    };
    let missing = |k: &str| env::var_os(k).is_none_or(|v| v.is_empty());

    if missing("XDG_RUNTIME_DIR") {
        // SAFETY: getuid has no preconditions.
        set("XDG_RUNTIME_DIR", &format!("/run/user/{}", unsafe { libc::getuid() }));
    }
    let run = env::var("XDG_RUNTIME_DIR").unwrap_or_default();
    if missing("WAYLAND_DISPLAY") {
        if let Some(name) = first_socket(&run, |n| n.strip_prefix("wayland-").is_some_and(is_digits)) {
            set("WAYLAND_DISPLAY", &name);
        }
    }
    if missing("DBUS_SESSION_BUS_ADDRESS") {
        set("DBUS_SESSION_BUS_ADDRESS", &format!("unix:path={run}/bus"));
    }
    if missing("NIRI_SOCKET") {
        if let Some(name) =
            first_socket(&run, |n| n.starts_with("niri.") && Path::new(n).extension().is_some_and(|e| e == "sock"))
        {
            set("NIRI_SOCKET", &format!("{run}/{name}"));
        }
    }
}

fn is_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// The alphabetically first socket in `dir` whose name matches.
fn first_socket(dir: &str, want: impl Fn(&str) -> bool) -> Option<String> {
    use std::os::unix::fs::FileTypeExt;
    let mut names: Vec<String> = std::fs::read_dir(Path::new(dir))
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_socket()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| want(n))
        .collect();
    names.sort();
    names.into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digits() {
        assert!(is_digits("0"));
        assert!(is_digits("12"));
        assert!(!is_digits(""));
        assert!(!is_digits("1.lock"));
    }

    #[test]
    fn finds_sockets_only() {
        let dir = std::env::temp_dir().join(format!("walkie-env-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("wayland-0.lock"), "").unwrap();
        std::fs::write(dir.join("wayland-0"), "").unwrap(); // a file, not a socket
        let _l = std::os::unix::net::UnixListener::bind(dir.join("wayland-1")).unwrap();
        let d = dir.to_str().unwrap();
        assert_eq!(
            first_socket(d, |n| n.strip_prefix("wayland-").is_some_and(is_digits)).as_deref(),
            Some("wayland-1")
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
