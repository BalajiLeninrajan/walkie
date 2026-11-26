//! Settings from `~/.config/walkie/config`.
//!
//! The file uses the same `KEY=value` shell syntax as the bash version, so an
//! existing config keeps working. Supported: comments, `export KEY=...`,
//! single quotes (literal), double quotes and bare words with `$VAR` and
//! `${VAR}` expansion. `$DATA` is the data directory, and keys set earlier in
//! the file can be referenced by later ones. Nothing is executed.

use std::collections::HashMap;
use std::env;
use std::path::PathBuf;

use crate::error::{Error, Result};

/// Where walkie keeps its files.
#[derive(Debug, Clone)]
pub struct Dirs {
    /// `~/.local/share/walkie`: models and sounds.
    pub data: PathBuf,
    /// `~/.local/state/walkie`: history.
    pub state: PathBuf,
    /// `$XDG_RUNTIME_DIR/walkie`: socket, state file, log.
    pub run: PathBuf,
    /// `~/.config/walkie/config`.
    pub config_file: PathBuf,
}

impl Dirs {
    /// Reads the XDG variables. Call after `env::fill_session_env`.
    pub fn from_env() -> Self {
        let home = env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from);
        let xdg = |var: &str, fallback: &str| {
            env::var_os(var).filter(|v| !v.is_empty()).map_or_else(|| home.join(fallback), PathBuf::from)
        };
        let run = env::var_os("XDG_RUNTIME_DIR").map_or_else(|| PathBuf::from("/tmp"), PathBuf::from);
        Self {
            data: xdg("XDG_DATA_HOME", ".local/share").join("walkie"),
            state: xdg("XDG_STATE_HOME", ".local/state").join("walkie"),
            run: run.join("walkie"),
            config_file: xdg("XDG_CONFIG_HOME", ".config").join("walkie/config"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeMode {
    /// Type the text through a virtual keyboard.
    Type,
    /// Put the text on the clipboard and press Ctrl+V (Ctrl+Shift+V in terminals).
    Paste,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub model: PathBuf,
    pub threads: u32,
    /// A tap shorter than this locks the recording. 0 disables locking.
    pub lock_ms: u64,
    /// Keep recording this long after release so the last word isn't cut.
    pub tail_ms: u64,
    /// Stop on our own after this many seconds, in case the release is lost.
    pub max_seconds: u64,
    pub sounds: bool,
    pub sound_dir: PathBuf,
    pub sound_volume: f32,
    pub notify: bool,
    pub type_mode: TypeMode,
    /// niri `app_id`s that get Ctrl+Shift+V in paste mode.
    pub terminals: Vec<String>,
    /// Appended after each transcript.
    pub trailing: String,
    /// Write numbers as digits ("twenty five" → "25").
    pub digits: bool,
    /// Optional filter command run with `sh -c`: transcript on stdin, text on stdout.
    pub postproc: String,
}

impl Config {
    pub fn defaults(dirs: &Dirs) -> Self {
        Self {
            model: dirs.data.join("models/parakeet-tdt_ctc-110m-Q8_0.gguf"),
            threads: 4,
            lock_ms: 300,
            tail_ms: 150,
            max_seconds: 300,
            sounds: true,
            sound_dir: dirs.data.join("sounds"),
            sound_volume: 0.4,
            notify: true,
            type_mode: TypeMode::Type,
            terminals: "foot|kitty|Alacritty|alacritty|ghostty|wezterm|herdr|konsole|gnome-terminal"
                .split('|')
                .map(str::to_owned)
                .collect(),
            trailing: " ".into(),
            digits: false,
            postproc: String::new(),
        }
    }

    /// Defaults, overridden by the config file if it exists. Returns the
    /// config and any warnings (unknown keys).
    pub fn load(dirs: &Dirs) -> Result<(Self, Vec<String>)> {
        let mut cfg = Self::defaults(dirs);
        let text = match std::fs::read_to_string(&dirs.config_file) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((cfg, Vec::new())),
            Err(source) => {
                return Err(Error::Io { what: format!("reading {}", dirs.config_file.display()), source });
            }
        };
        let data = dirs.data.to_string_lossy().into_owned();
        let lookup = |name: &str| if name == "DATA" { Some(data.clone()) } else { env::var(name).ok() };
        let warnings = cfg.apply(&text, lookup).map_err(|(line, msg)| Error::Config {
            path: dirs.config_file.clone(),
            line,
            msg,
        })?;
        Ok((cfg, warnings))
    }

    /// Applies `KEY=value` lines on top of `self`. `lookup` resolves `$VARS`
    /// that the file itself hasn't set. Errors carry the 1-based line number.
    pub fn apply(
        &mut self,
        text: &str,
        lookup: impl Fn(&str) -> Option<String>,
    ) -> std::result::Result<Vec<String>, (usize, String)> {
        let mut set: HashMap<String, String> = HashMap::new();
        let mut warnings = Vec::new();
        for (i, raw) in text.lines().enumerate() {
            let line_no = i + 1;
            let Some((key, value)) = parse_line(raw, |name| set.get(name).cloned().or_else(|| lookup(name)))
                .map_err(|msg| (line_no, msg))?
            else {
                continue;
            };
            if let Some(w) = self.set(&key, &value).map_err(|msg| (line_no, format!("{key}: {msg}")))? {
                warnings.push(format!("line {line_no}: {w}"));
            }
            set.insert(key, value);
        }
        Ok(warnings)
    }

    /// Sets one key. Returns a warning for keys this version doesn't use.
    fn set(&mut self, key: &str, v: &str) -> std::result::Result<Option<String>, String> {
        match key {
            "MODEL" => self.model = PathBuf::from(v),
            "THREADS" => self.threads = parse_num(v)?,
            "LOCK_MS" => self.lock_ms = parse_num(v)?,
            "TAIL_MS" => self.tail_ms = parse_num(v)?,
            "MAX_SECONDS" => self.max_seconds = parse_num(v)?,
            "SOUNDS" => self.sounds = parse_bool(v)?,
            "SOUND_DIR" => self.sound_dir = PathBuf::from(v),
            "SOUND_VOLUME" => {
                self.sound_volume = v.parse().map_err(|_| format!("not a number: {v:?}"))?;
                if !(0.0..=1.0).contains(&self.sound_volume) {
                    return Err("must be between 0 and 1".into());
                }
            }
            "NOTIFY" => self.notify = parse_bool(v)?,
            "TYPE_MODE" => {
                self.type_mode = match v {
                    "type" => TypeMode::Type,
                    "paste" => TypeMode::Paste,
                    _ => return Err(format!("expected type or paste, got {v:?}")),
                }
            }
            "TERMINALS" => self.terminals = v.split('|').filter(|s| !s.is_empty()).map(str::to_owned).collect(),
            "TRAILING" => v.clone_into(&mut self.trailing),
            "DIGITS" => self.digits = parse_bool(v)?,
            "POSTPROC" => v.clone_into(&mut self.postproc),
            // The bash version's helper path; the Rust build has no helper.
            "PIPE_BIN" => {}
            _ => return Ok(Some(format!("unknown setting {key}, ignored"))),
        }
        Ok(None)
    }
}

fn parse_num<T: std::str::FromStr>(v: &str) -> std::result::Result<T, String> {
    v.parse().map_err(|_| format!("not a whole number: {v:?}"))
}

fn parse_bool(v: &str) -> std::result::Result<bool, String> {
    match v {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" | "" => Ok(false),
        _ => Err(format!("expected 1 or 0, got {v:?}")),
    }
}

/// Parses one line. `Ok(None)` for blank lines and comments.
fn parse_line(
    line: &str,
    lookup: impl Fn(&str) -> Option<String>,
) -> std::result::Result<Option<(String, String)>, String> {
    let line = line.trim_start();
    if line.is_empty() || line.starts_with('#') {
        return Ok(None);
    }
    let line = line.strip_prefix("export ").map_or(line, str::trim_start);
    let (key, rest) = line.split_once('=').ok_or("expected KEY=value")?;
    if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(format!("bad key {key:?}"));
    }
    Ok(Some((key.to_owned(), parse_value(rest, &lookup)?)))
}

/// Shell-style word parsing for the right-hand side of an assignment.
fn parse_value(s: &str, lookup: &impl Fn(&str) -> Option<String>) -> std::result::Result<String, String> {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' => loop {
                match chars.next() {
                    Some('\'') => break,
                    Some(c) => out.push(c),
                    None => return Err("unterminated single quote".into()),
                }
            },
            '"' => loop {
                match chars.next() {
                    Some('"') => break,
                    Some('\\') => match chars.next() {
                        Some(c @ ('"' | '\\' | '$' | '`')) => out.push(c),
                        Some(c) => {
                            out.push('\\');
                            out.push(c);
                        }
                        None => return Err("unterminated double quote".into()),
                    },
                    Some('$') => expand(&mut chars, &mut out, lookup)?,
                    Some(c) => out.push(c),
                    None => return Err("unterminated double quote".into()),
                }
            },
            '\\' => out.extend(chars.next()),
            '$' => expand(&mut chars, &mut out, lookup)?,
            c if c.is_whitespace() => {
                let rest: String = chars.collect();
                let rest = rest.trim_start();
                if rest.is_empty() || rest.starts_with('#') {
                    break;
                }
                return Err("unquoted space in value; put the value in quotes".into());
            }
            '`' => return Err("command substitution is not supported".into()),
            c => out.push(c),
        }
    }
    Ok(out)
}

/// Expands the variable after a `$`.
fn expand(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    out: &mut String,
    lookup: &impl Fn(&str) -> Option<String>,
) -> std::result::Result<(), String> {
    let mut name = String::new();
    if chars.peek() == Some(&'{') {
        chars.next();
        loop {
            match chars.next() {
                Some('}') => break,
                Some(c) if c.is_ascii_alphanumeric() || c == '_' => name.push(c),
                Some(c) => return Err(format!("unsupported character {c:?} in ${{...}}")),
                None => return Err("unterminated ${".into()),
            }
        }
    } else if chars.peek() == Some(&'(') {
        return Err("command substitution is not supported".into());
    } else {
        while let Some(&c) = chars.peek() {
            if !(c.is_ascii_alphanumeric() || c == '_') {
                break;
            }
            name.push(c);
            chars.next();
        }
    }
    if name.is_empty() {
        out.push('$');
    } else {
        out.push_str(&lookup(&name).unwrap_or_default());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs() -> Dirs {
        Dirs { data: "/d".into(), state: "/s".into(), run: "/r".into(), config_file: "/c".into() }
    }

    fn apply(text: &str) -> std::result::Result<(Config, Vec<String>), (usize, String)> {
        let mut cfg = Config::defaults(&dirs());
        let lookup = |n: &str| match n {
            "HOME" => Some("/home/u".to_owned()),
            "DATA" => Some("/d".to_owned()),
            _ => None,
        };
        let w = cfg.apply(text, lookup)?;
        Ok((cfg, w))
    }

    #[test]
    fn empty_file_keeps_defaults() {
        let (cfg, w) = apply("\n# comment\n   \n").unwrap();
        assert_eq!(cfg, Config::defaults(&dirs()));
        assert!(w.is_empty());
    }

    #[test]
    fn bash_style_config_parses() {
        let text = r#"
MODEL=$DATA/models/parakeet-unified-en-0.6b-Q8_0.gguf
export THREADS=2   # fewer threads
LOCK_MS=0
TYPE_MODE=paste
TRAILING=" "
TERMINALS='foot|kitty'
SOUND_DIR="${HOME}/sounds"
POSTPROC="$DATA/py/bin/python $DATA/bin/dictate-numbers"
"#;
        let (cfg, w) = apply(text).unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(cfg.model, PathBuf::from("/d/models/parakeet-unified-en-0.6b-Q8_0.gguf"));
        assert_eq!(cfg.threads, 2);
        assert_eq!(cfg.lock_ms, 0);
        assert_eq!(cfg.type_mode, TypeMode::Paste);
        assert_eq!(cfg.trailing, " ");
        assert_eq!(cfg.terminals, ["foot", "kitty"]);
        assert_eq!(cfg.sound_dir, PathBuf::from("/home/u/sounds"));
        assert_eq!(cfg.postproc, "/d/py/bin/python /d/bin/dictate-numbers");
    }

    #[test]
    fn later_keys_see_earlier_ones() {
        let (cfg, _) = apply("SOUND_DIR=/x\nMODEL=$SOUND_DIR/m.gguf\n").unwrap();
        assert_eq!(cfg.model, PathBuf::from("/x/m.gguf"));
    }

    #[test]
    fn quoting_rules() {
        let q = |s: &str| parse_value(s, &|_: &str| Some("V".to_owned()));
        assert_eq!(q(r"'$A b'").unwrap(), "$A b");
        assert_eq!(q(r#""$A \"b\" \$c""#).unwrap(), r#"V "b" $c"#);
        assert_eq!(q(r"a\ b").unwrap(), "a b");
        assert_eq!(q("a$").unwrap(), "a$");
        assert_eq!(q("x # trailing comment").unwrap(), "x");
        assert!(q("a b").is_err());
        assert!(q("'open").is_err());
        assert!(q("$(id)").is_err());
        assert!(q("`id`").is_err());
    }

    #[test]
    fn bad_values_report_the_line() {
        let (line, msg) = apply("\nTHREADS=four\n").unwrap_err();
        assert_eq!(line, 2);
        assert!(msg.contains("THREADS"), "{msg}");
        assert_eq!(apply("TYPE_MODE=xdotool").unwrap_err().0, 1);
        assert_eq!(apply("SOUND_VOLUME=2").unwrap_err().0, 1);
        assert_eq!(apply("just words").unwrap_err().0, 1);
    }

    #[test]
    fn unknown_keys_warn() {
        let (_, w) = apply("FOO=1\nPIPE_BIN=/x\nDIGITS=1").unwrap();
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("FOO"));
    }

    #[test]
    fn booleans() {
        let (cfg, _) = apply("SOUNDS=0\nNOTIFY=no\nDIGITS=true").unwrap();
        assert!(!cfg.sounds && !cfg.notify && cfg.digits);
    }
}
