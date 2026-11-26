//! `history.tsv`: one `timestamp<TAB>transcript` line per dictation.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use crate::error::{IoContext, Result};

const FILE: &str = "history.tsv";

pub fn append(state_dir: &Path, text: &str) -> Result<()> {
    fs::create_dir_all(state_dir).ctx(|| format!("creating {}", state_dir.display()))?;
    let path = state_dir.join(FILE);
    let line = format!("{}\t{}\n", local_timestamp(), text.replace(['\t', '\n'], " "));
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut f| f.write_all(line.as_bytes()))
        .ctx(|| format!("writing {}", path.display()))
}

/// The most recent transcript, if any.
pub fn last(state_dir: &Path) -> Option<String> {
    let text = fs::read_to_string(state_dir.join(FILE)).ok()?;
    last_entry(&text)
}

fn last_entry(history: &str) -> Option<String> {
    history.lines().rev().find_map(|l| l.split_once('\t')).map(|(_, t)| t.to_owned())
}

/// Local time like `date -Is`: `2026-11-26T13:05:09-05:00`.
fn local_timestamp() -> String {
    // SAFETY: time(NULL) and localtime_r with valid pointers have no other
    // preconditions; `tm` is plain data that localtime_r fills in.
    let tm = unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&raw const t, &raw mut tm);
        tm
    };
    format_timestamp(&tm)
}

fn format_timestamp(tm: &libc::tm) -> String {
    let off = tm.tm_gmtoff / 60;
    let sign = if off < 0 { '-' } else { '+' };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{sign}{:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec,
        off.abs() / 60,
        off.abs() % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_entry_takes_the_final_line() {
        let h = "2026-01-01T00:00:00+00:00\tfirst\n2026-01-01T00:00:01+00:00\tsecond one\n";
        assert_eq!(last_entry(h).as_deref(), Some("second one"));
        assert_eq!(last_entry(""), None);
    }

    #[test]
    fn timestamp_format() {
        // SAFETY: an all-zero tm is a valid value.
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        tm.tm_year = 126;
        tm.tm_mon = 10;
        tm.tm_mday = 26;
        tm.tm_hour = 9;
        tm.tm_min = 5;
        tm.tm_sec = 7;
        tm.tm_gmtoff = -5 * 3600;
        assert_eq!(format_timestamp(&tm), "2026-11-26T09:05:07-05:00");
        tm.tm_gmtoff = 5 * 3600 + 1800;
        assert_eq!(format_timestamp(&tm), "2026-11-26T09:05:07+05:30");
    }

    #[test]
    fn append_and_read_back() {
        let dir = std::env::temp_dir().join(format!("walkie-hist-{}", std::process::id()));
        append(&dir, "hello\tthere").unwrap();
        append(&dir, "again").unwrap();
        assert_eq!(last(&dir).as_deref(), Some("again"));
        let all = fs::read_to_string(dir.join(FILE)).unwrap();
        assert!(all.lines().next().unwrap().ends_with("\thello there"));
        fs::remove_dir_all(dir).unwrap();
    }
}
