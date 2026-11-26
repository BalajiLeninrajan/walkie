//! Turning the model's output into the text that gets typed.

use std::io::Write;
use std::process::{Command, Stdio};

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

/// Runs the user's `POSTPROC` command. On failure the text passes through
/// unchanged and the error is returned alongside it.
pub fn postproc(cmd: &str, text: &str) -> (String, Option<String>) {
    let run = || -> std::io::Result<std::process::Output> {
        let mut child = Command::new("sh")
            .args(["-c", cmd])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        child.stdin.take().expect("piped stdin").write_all(text.as_bytes())?;
        child.wait_with_output()
    };
    match run() {
        Ok(out) if out.status.success() => (normalize(&String::from_utf8_lossy(&out.stdout)), None),
        Ok(out) => (
            text.to_owned(),
            Some(format!("POSTPROC exited with {}: {}", out.status, String::from_utf8_lossy(&out.stderr).trim())),
        ),
        Err(e) => (text.to_owned(), Some(format!("POSTPROC failed to run: {e}"))),
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
        assert_eq!(postproc("tr a-z A-Z", "hello"), ("HELLO".to_owned(), None));
        let (out, err) = postproc("exit 3", "hello");
        assert_eq!(out, "hello");
        assert!(err.unwrap().contains("exit status: 3"));
    }
}
