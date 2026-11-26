# Walkie DMS plugin

A [DankMaterialShell](https://github.com/AvengeMedia/DankMaterialShell) plugin that shows a small pill near the bottom of the screen while walkie is dictating.

- While recording, the pill shows a mic and a level meter that follows your voice. Once a tap locks the recording, the mic turns into a lock and the pill gets an outline, so you know another press finishes it.
- While transcribing, it shows three moving dots and "Transcribing".
- Otherwise it's hidden, and no window exists. The pill never takes keyboard focus and clicks go through it.

The plugin only reads the state file walkie writes, `$XDG_RUNTIME_DIR/walkie/state`. It needs no flags on walkie and talks to it no other way.

## Setup

Link this folder into the plugins directory, then enable **Walkie** under Settings → Plugins:

```sh
ln -s ~/Documents/code/walkie/plugin ~/.config/dms/plugins/walkie
```

DMS 1.x read plugins from `~/.config/DankMaterialShell/plugins/` instead. This plugin targets DMS 2.1 and later.

You can also enable it from a shell:

```sh
dms ipc call plugin-scan scan
dms ipc call plugins enable walkie
```

## Settings

| Setting | Default | |
|---|---|---|
| Bottom margin | 64 px | Distance from the bottom edge of the screen to the pill |
| Screens | All screens | Show the pill on every screen, or only on the one with focus |
| State file | `$XDG_RUNTIME_DIR/walkie/state` | Where walkie writes its state |

## How it reads the state file

The file holds one line such as `v=1 state=recording level=0.42 locked=0`. walkie rewrites it in place every 40 ms while recording and every 250 ms otherwise, and leaves it at `idle` between dictations.

- If the file doesn't exist yet, the plugin checks again every 2 s until it shows up.
- If a non-idle state goes 2 s without a rewrite, the plugin assumes walkie died and hides the pill.
- The plugin reads format version 1. For any other version it shows nothing and logs one warning, so update walkie or the plugin to match. Unknown keys are ignored.

## Limits

DMS logs "Created graphical object was not placed in the graphics scene" when the plugin loads. Every daemon plugin gets it, because DMS creates the plugin's root item without a window, and it's harmless.
