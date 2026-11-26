# walkie

over.

Push-to-talk dictation for Wayland. Hold a key, talk, let go, and the text is typed into the focused window. Nothing stays in memory between dictations: each press starts a short-lived process that records, transcribes and types, then exits.

- **Engine.** [transcribe.cpp](https://github.com/handy-computer/transcribe.cpp) (the engine Hex uses) with Parakeet TDT-CTC 110M, on the CPU. The model loads while you talk, so only the transcription is left after you let go. That's 0.7 to 1 s for a 10 to 15 s dictation, on 4 threads of a Ryzen 7 5700U capped at 1.8 GHz.
- **Memory.** About 270 MB during a dictation (half of it the memory-mapped model file), and none between dictations.
- **Native.** One binary. It records through PipeWire, types through the Wayland virtual-keyboard protocol (any Unicode character), plays its beeps through PipeWire, and converts spoken numbers to digits with [text2num](https://crates.io/crates/text2num). The only helpers it spawns are `notify-send` and, in paste mode, `wl-copy`.
- **Indicator.** An optional [DankMaterialShell](https://github.com/AvengeMedia/DankMaterialShell) plugin shows a pill at the bottom of the screen with a live mic level while you talk.

## How it works

`walkie start` runs on key press. It binds a control socket at `$XDG_RUNTIME_DIR/walkie/sock`, forks and returns at once. The child is the session:

1. It opens the microphone, starts loading the model on a second thread, beeps, and starts writing the state file for the indicator.
2. `walkie stop` (key release) sends `release` over the socket. The session keeps recording for `TAIL_MS` so the last word isn't cut, then stops.
3. It transcribes, cleans up the text (optional digits and `POSTPROC` filter), appends it to the history, types it, and exits.

A tap shorter than `LOCK_MS` locks the recording instead, for hands-free dictation. The next press-and-release ends it. Text is only ever typed on a key release, so the push-to-talk key is never held while keystrokes arrive.

- **Overlapping dictations.** If you press again while the last dictation is still transcribing, a second session starts. The first one waits for the new key to come up before it types, so the texts arrive in order.
- **Release timing.** `start` and `stop` stamp the key event with the time kanata ran them. They take a lock while they look for a session, so a very quick tap is measured correctly even if the release reaches the session first.

## Install

walkie runs on Fedora 43 and 44 on x86_64, on any CPU with AVX2, FMA, F16C and BMI2 (Haswell or newer). As your user:

```sh
curl -fsSL https://raw.githubusercontent.com/BalajiLeninrajan/walkie/main/install.sh | sh
```

This needs no compiler and no sudo. `install.sh` does the following:

1. It asks the GitHub API for the latest release, downloads `walkie-<version>-x86_64-linux.tar.gz` and `SHA256SUMS`, and checks the tarball against `SHA256SUMS`. `WALKIE_VERSION=v0.2.0` picks a release instead.
2. It installs the binary to `~/.local/bin/walkie`.
3. It downloads [parakeet-tdt_ctc-110m-Q8_0.gguf](https://huggingface.co/handy-computer/parakeet-tdt_ctc-110m-gguf) (130 MB) into `~/.local/share/walkie/models/` and checks its SHA-256. A model that's already there and matches isn't downloaded again. The `unified` argument also fetches Hex's 0.6B model: `curl ... | sh -s -- unified`.
4. It copies `plugin/` to `~/.config/dms/plugins/walkie` and enables it with `dms ipc call plugins enable walkie`. DMS 1.x used `~/.config/DankMaterialShell/plugins/` instead, and the script uses that path when only the old directory exists.
5. It checks for `libpipewire-0.3.so.0`, `notify-send` and `wl-copy`, which a Fedora desktop usually has. It doesn't install them. If any are missing, it prints the command to run, such as `sudo dnf install libnotify wl-clipboard`.

Run it again to upgrade.

To install a release by hand, download the tarball and `SHA256SUMS` from the [releases page](https://github.com/BalajiLeninrajan/walkie/releases), then:

```sh
sha256sum -c --ignore-missing SHA256SUMS
tar xzf walkie-0.1.0-x86_64-linux.tar.gz
cd walkie-0.1.0-x86_64-linux
./install.sh
```

Run from an unpacked tarball, `install.sh` installs the files next to it and only downloads the model.

### From source

```sh
git clone --recurse-submodules https://github.com/BalajiLeninrajan/walkie ~/Documents/code/walkie
cd ~/Documents/code/walkie
./install.sh --from-source
```

In a checkout, `install.sh` without `--from-source` still installs the latest release. `--from-source` replaces the download with a build:

1. `sudo dnf install gcc gcc-c++ cmake make git pkgconf-pkg-config pipewire-devel wl-clipboard libnotify`, plus `cargo rust` if you don't use rustup. Rust 1.85 or newer is required. `SKIP_DNF=1` skips this step.
2. `cargo build --release --locked`, with `CARGO_BUILD_JOBS=4` so the transcribe.cpp build doesn't run a small laptop out of memory. It installs `target/release/walkie` to `~/.local/bin/walkie`.

The model and plugin steps are the same as above.

ggml is compiled with `-march=native` for the machine that builds it. To build a binary for another machine, set `WALKIE_GGML_NATIVE=0`, which targets any x86-64 CPU with SSE4.2, AVX2, FMA, F16C and BMI2. Release binaries are built this way.

## Kanata

Run `walkie start` on press and `walkie stop` on release. Use full paths, since kanata's `PATH` may not include `~/.local/bin`. kanata must run as your user, not root. walkie fills in `XDG_RUNTIME_DIR`, `WAYLAND_DISPLAY`, `DBUS_SESSION_BUS_ADDRESS` and `NIRI_SOCKET` when kanata's environment lacks them.

If you already have a key wired to press and release commands (for Hex, say), swap in the two walkie commands. Otherwise, for example on right Super:

```
(defcfg danger-enable-cmd yes)
(defvirtualkeys walkie-stop (cmd /home/you/.local/bin/walkie stop))
(defalias walkie (multi (cmd /home/you/.local/bin/walkie start) (on-release tap-vkey walkie-stop)))
```

Other commands:

| Command | What it does |
|---|---|
| `walkie cancel` | Drop the current recording without typing anything |
| `walkie last` | Print the last transcript |
| `walkie repaste` | Type the last transcript again |
| `walkie status` | Print the indicator state record |

## Config

`~/.config/walkie/config` uses `KEY=value` lines in shell syntax. A config from the bash `dictate` script works unchanged. The file is parsed, never executed:

- Supported: comments, `export`, single and double quotes, and `$VAR`, `${VAR}` and `${VAR:-default}`.
- `$DATA` is `~/.local/share/walkie`, and later keys can use earlier ones.
- Command substitution is rejected.

| Key | Default | |
|---|---|---|
| `MODEL` | `$DATA/models/parakeet-tdt_ctc-110m-Q8_0.gguf` | Any transcribe.cpp GGUF |
| `THREADS` | `4` | Inference threads |
| `LOCK_MS` | `300` | A tap shorter than this locks the recording. `0` disables locking |
| `TAIL_MS` | `150` | Keep recording this long after release |
| `MAX_SECONDS` | `300` | Finish on our own if the release never arrives. A session still running 120 s after that is ended |
| `DIGITS` | `0` | `1` writes numbers as digits: "twenty five" becomes "25". Isolated "one" and "two" stay words |
| `POSTPROC` | empty | Filter command run with `sh -c`, transcript on stdin. Killed after 10 s, and the text passes through unchanged |
| `TYPE_MODE` | `type` | `paste` copies with `wl-copy` and presses Ctrl+V, or Ctrl+Shift+V in terminals |
| `TERMINALS` | `foot\|kitty\|Alacritty\|alacritty\|ghostty\|wezterm\|herdr\|konsole\|gnome-terminal` | niri `app_id`s that get Ctrl+Shift+V. Exact names separated by `\|` |
| `KEY_DELAY_MS` | `0` | Pause between typed keys, if an app drops keys |
| `TRAILING` | `" "` | Appended after each transcript |
| `SOUNDS` | `1` | Beeps on start, stop, lock and error |
| `SOUND_DIR` | `$DATA/sounds` | `start.wav`, `stop.wav`, `lock.wav`, `error.wav` override the built-in beeps (16-bit PCM) |
| `SOUND_VOLUME` | `0.4` | 0 to 1 |
| `NOTIFY` | `1` | Desktop notifications for each step. With the DMS indicator you'll probably want `0`. Errors still notify |

Files:

- `~/.local/state/walkie/history.tsv`: one `timestamp<TAB>text` line per dictation.
- `$XDG_RUNTIME_DIR/walkie/log`: the last session's log, with timings.
- `$XDG_RUNTIME_DIR/walkie/state`: the indicator state file, described below.

## DMS plugin

`plugin/` is a DMS daemon plugin with id `walkie`. While you dictate, it shows a pill near the bottom center of the screen:

- a mic and a live level meter while recording
- a lock while a tap has locked the recording
- a "transcribing" state after release

It never takes keyboard focus and the pill is click-through. It uses the DMS theme colors. walkie pushes each state record to the plugin over a socket. Measured on a Ryzen 7 5700U at 1.8 GHz, the plugin adds:

- nothing measurable to DMS's CPU while idle
- about 6% of a core while the pill is showing the live meter, and 2 to 3% while transcribing
- about 8 MB of memory while it shows

See [plugin/README.md](plugin/README.md) for its settings.

## State file

`$XDG_RUNTIME_DIR/walkie/state` is the contract between `walkie` and anything that wants to show its state. It holds one line of space-separated `key=value` fields, padded with spaces to exactly 63 bytes plus a newline:

```
v=1 state=recording level=0.42 locked=0
```

| Field | Values |
|---|---|
| `v` | Format version, currently `1`. |
| `state` | `idle`, `recording` or `transcribing` |
| `level` | Mic level 0.00 to 1.00, from -60 to 0 dBFS (peak RMS since the last update). 0.00 outside `recording` |
| `locked` | `1` if a tap locked the recording |

Rules:

- `v` changes when a field changes meaning or is removed. New fields can be added without a bump, so readers must ignore keys they don't know. A reader that sees an unsupported `v` should show nothing and say so; the plugin logs a warning.
- Each update rewrites the whole record in place at offset 0 with one `pwrite`. The file never shrinks and its inode never changes, so inotify watches stay valid and a reader never sees an empty file.
- Updates arrive every 40 ms while recording, and at least every 250 ms while a session is running. A non-idle state that hasn't been rewritten for 2 s means the session died, and readers should treat it as idle.
- The file stays in place, saying `idle`, between dictations. It doesn't exist before the first dictation after boot.
- If something listens on `$XDG_RUNTIME_DIR/walkie-indicator.sock`, walkie connects at the start of each dictation and sends every record there too, one per line. This is how the DMS plugin gets its updates. A listener that can't keep up loses records, never walkie's time. Re-reading the file on every inotify event cost DMS about 7% of a core at 25 updates a second; the socket costs about 3%.
- When dictations overlap, the newest session owns the file and the stream. Older sessions stop writing once a newer one exists.

## Development

```sh
git submodule update --init
cargo build --release
cargo test
cargo clippy --all-targets -- -D warnings   # pedantic lints are on in Cargo.toml
cargo fmt --check
```

CI (`.github/workflows/ci.yml`) runs the last three in a `fedora:43` container on pushes to main and on pull requests.

The layout:

| Path | |
|---|---|
| `src/main.rs` | CLI |
| `src/session.rs` | The forked session: record, wait for release, transcribe, type |
| `src/machine.rs` | Press/release/lock state machine |
| `src/config.rs` | Config file parser |
| `src/audio.rs`, `csrc/pw.c` | PipeWire capture and beeps |
| `src/engine.rs`, `csrc/engine.c` | transcribe.cpp |
| `src/keyboard.rs`, `src/output.rs` | Virtual keyboard, paste mode, niri focus query |
| `src/indicator.rs` | State file writer |
| `src/ipc.rs`, `src/notify.rs`, `src/history.rs`, `src/text.rs`, `src/sound.rs`, `src/session_env.rs` | The rest |
| `plugin/` | DMS plugin |
| `scripts/dist.sh` | Builds and packages a release |
| `.github/workflows/` | CI and releases |
| `vendor/transcribe.cpp` | Submodule, pinned to a0729fa |

The C shims keep transcribe.cpp's and PipeWire's structs on the C side, so there are no hand-written struct layouts in Rust. `build.rs` builds transcribe.cpp's static libraries with CMake. Set `TRANSCRIBE_DIR` to use another checkout.

Two environment variables exist for benchmarks:

- `WALKIE_TEST_WAV=clip.wav` feeds a 16 kHz WAV in real time instead of the microphone, then silence until release.
- `WALKIE_TEST_OUT=file` appends `<unix ms> <text>` to a file instead of typing.

## Releasing

`.github/workflows/release.yml` builds releases. It runs `scripts/dist.sh` in a `fedora:43` container, the oldest Fedora walkie supports, so the binary needs no newer glibc or libpipewire than that. ggml is built with `WALKIE_GGML_NATIVE=0`, and the script fails if the binary contains AVX-512 instructions.

1. Bump `version` in `Cargo.toml`, and in `plugin/plugin.json` if the plugin changed. Run `cargo build` so `Cargo.lock` picks up the new version, since the release builds with `--locked`. Commit.
2. Tag the commit and push the tag:

   ```sh
   git tag v0.2.0
   git push origin main v0.2.0
   ```

The workflow fails if the tag doesn't match the version in `Cargo.toml`. Otherwise it publishes a GitHub release with the tarball, `SHA256SUMS`, and notes listing the commits since the previous tag.

To try it without releasing, run the workflow by hand from the Actions tab or with `gh workflow run release.yml`. That run uploads the tarball, `SHA256SUMS` and the notes as an artifact of the run. `scripts/dist.sh` builds the tarball into `dist/` on your own machine, though only a `fedora:43` container gives you the same glibc and toolchain as the release.

## License

MIT, see [LICENSE](LICENSE).
