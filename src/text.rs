//! Turning the model's output into the text that gets typed.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use text2num::{Language, replace_numbers_in_text};

/// Collapses newlines and runs of whitespace into single spaces and trims.
pub fn normalize(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Rewrites spoken numbers as digits: "twenty five" becomes "25". Isolated
/// numbers below 3 ("one", "two") stay words, same as the Python filter's
/// `alpha2digit(text, "en", threshold=3)`.
pub fn digits(text: &str) -> String {
    replace_numbers_in_text(text, &Language::english(), 3.0)
}

/// Runs the user's `POSTPROC` command, killing it after `timeout`. On any
/// failure the text passes through unchanged and the error comes back
/// alongside it.
pub fn postproc(cmd: &str, text: &str, timeout: Duration) -> (String, Option<String>) {
    match run_filter(cmd, text, timeout) {
        Ok(out) => (normalize(&out), None),
        Err(e) => (text.to_owned(), Some(format!("POSTPROC: {e}"))),
    }
}

fn run_filter(cmd: &str, text: &str, timeout: Duration) -> Result<String, String> {
    let mut child = Command::new("sh")
        .args(["-c", cmd])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not run: {e}"))?;
    // Transcripts are far below the pipe buffer size, so this can't block.
    let _ = child.stdin.take().expect("piped stdin").write_all(text.as_bytes());
    let drain = |mut r: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = r.read_to_end(&mut buf);
            String::from_utf8_lossy(&buf).into_owned()
        })
    };
    let stdout = drain(Box::new(child.stdout.take().expect("piped stdout")));
    let stderr = drain(Box::new(child.stderr.take().expect("piped stderr")));
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                // The reader threads may stay blocked on a grandchild that
                // kept the pipes open; the session exits soon anyway.
                return Err(format!("timed out after {} s", timeout.as_secs()));
            }
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    };
    let out = stdout.join().unwrap_or_default();
    if status.success() {
        Ok(out)
    } else {
        Err(format!("exited with {status}: {}", stderr.join().unwrap_or_default().trim()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_joins_lines() {
        assert_eq!(normalize("  hello\nworld \t again \n"), "hello world again");
        assert_eq!(normalize("\n \n"), "");
    }

    #[test]
    fn digits_converts_groups_and_keeps_small_isolated_numbers() {
        assert_eq!(digits("I have two cats and twenty five fish."), "I have two cats and 25 fish.");
        assert_eq!(digits("call me at five five five one two three four"), "call me at 5 5 5 1 2 3 4");
        assert_eq!(digits("It costs three hundred and twelve dollars."), "It costs 312 dollars.");
        assert_eq!(digits("pi is three point one four"), "pi is 3.14");
    }

    #[test]
    fn digits_leaves_plain_text_alone() {
        let s = "Ask not what your country can do for you.";
        assert_eq!(digits(s), s);
    }

    #[test]
    fn postproc_filters_and_falls_back() {
        let t = Duration::from_secs(5);
        assert_eq!(postproc("tr a-z A-Z", "hello", t), ("HELLO".to_owned(), None));
        let (out, err) = postproc("echo oops >&2; exit 3", "hello", t);
        assert_eq!(out, "hello");
        let err = err.unwrap();
        assert!(err.contains("exit status: 3") && err.contains("oops"), "{err}");
    }

    #[test]
    fn postproc_hang_is_killed() {
        let start = Instant::now();
        let (out, err) = postproc("exec sleep 30", "hello", Duration::from_millis(200));
        assert_eq!(out, "hello");
        assert!(err.unwrap().contains("timed out"));
        assert!(start.elapsed() < Duration::from_secs(5));
    }
}
