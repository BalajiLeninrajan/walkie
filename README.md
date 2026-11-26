# walkie

over.

Push-to-talk dictation for Wayland. Hold a key, talk, let go, and your words get typed into whatever window you're in.

It runs on your CPU, needs no internet once installed, and uses no memory when you're not talking. A 15 second dictation shows up about a second after you let go, even on a modest laptop.

## Install

You need:

- 64-bit Linux with glibc 2.39 or newer. Fedora 40, Ubuntu 24.04, Debian 13, Arch and anything newer all work.
- An Intel CPU from 2013 or later (Haswell) or an AMD one from 2015 or later (Excavator).
- A Wayland compositor with the virtual keyboard protocol. niri, Sway and Hyprland have it. GNOME doesn't.
- PipeWire.
- Optionally, `notify-send` for notifications and `wl-copy` for `TYPE_MODE=paste`. They usually come in packages called `libnotify` and `wl-clipboard`.

walkie doesn't install any of these for you. Then run:

```sh
curl -fsSL https://raw.githubusercontent.com/BalajiLeninrajan/walkie/main/install.sh | sh
```

No sudo needed. It downloads walkie and its 130 MB speech model. It stops before installing anything if your CPU, glibc or PipeWire won't work. Run the same command again to upgrade.

## Set up a key

walkie doesn't grab a key itself. Have your key remapper run `walkie start` when you press the key and `walkie stop` when you release it. With [kanata](https://github.com/jtroo/kanata), for example:

```
(defcfg danger-enable-cmd yes)
(defvirtualkeys walkie-stop (cmd /home/you/.local/bin/walkie stop))
(defalias walkie (multi (cmd /home/you/.local/bin/walkie start) (on-release tap-vkey walkie-stop)))
```

Use full paths, and run kanata as your user, not root.

Tap the key instead of holding it to go hands-free. Tap again when you're done.

## Commands

| Command | What it does |
|---|---|
| `walkie cancel` | Throw away the current recording |
| `walkie last` | Print the last thing you said |
| `walkie repaste` | Type the last thing you said again |

Everything you dictate is saved in `~/.local/state/walkie/history.tsv`.

## Settings

Put `KEY=value` lines in `~/.config/walkie/config`.

| Setting | Default | What it does |
|---|---|---|
| `DIGITS` | `0` | Set to `1` to write "twenty five" as "25" |
| `TYPE_MODE` | `type` | Set to `paste` to paste through the clipboard instead of typing |
| `SOUNDS` | `1` | Beeps when recording starts and stops |
| `SOUND_VOLUME` | `0.4` | Beep volume, 0 to 1 |
| `NOTIFY` | `1` | Desktop notifications. Errors always notify |
| `LOCK_MS` | `300` | How short a tap counts as hands-free. `0` turns it off |
| `TRAILING` | `" "` | Added after each dictation |
| `POSTPROC` | empty | A command to clean up the text before it's typed. It gets the text on stdin |

## On-screen indicator

If you use [DankMaterialShell](https://github.com/AvengeMedia/DankMaterialShell), the installer adds a small pill at the bottom of the screen that shows a mic level while you talk. Turn off `NOTIFY` if you use it. See [plugin/README.md](plugin/README.md) for its settings.

## Building and hacking

See [DEVELOPMENT.md](DEVELOPMENT.md).

## License

MIT, see [LICENSE](LICENSE).
