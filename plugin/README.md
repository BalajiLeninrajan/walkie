# Walkie DMS plugin

A [DankMaterialShell](https://github.com/AvengeMedia/DankMaterialShell) plugin that shows a small pill near the bottom of the screen while walkie is dictating.

- While recording, the pill shows a mic and a level meter that follows your voice. Once a tap locks the recording, the mic turns into a lock and the pill gets an outline, so you know another press finishes it.
- While transcribing, it shows three dots lit in turn and "Transcribing".
- Otherwise it's hidden, and no window exists. The pill never takes keyboard focus and clicks go through it.

walkie sends its state to the plugin over `$XDG_RUNTIME_DIR/walkie-indicator.sock`, which the plugin listens on. The plugin also reads walkie's state file, `$XDG_RUNTIME_DIR/walkie/state`, once when it starts. walkie needs no flags for either.

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

The scan finishes a moment after the command returns. If the enable answers `PLUGIN_NOT_FOUND`, run it again. `dms ipc` exits 0 either way, so read its reply.

## Settings

| Setting | Default | |
|---|---|---|
| Bottom margin | 64 px | Distance from the bottom edge of the screen to the pill |
| Screens | All screens | Show the pill on every screen, or only on the one with focus |
| State file | `$XDG_RUNTIME_DIR/walkie/state` | Where walkie writes its state; read once when the plugin starts |

## How it gets walkie's state

Each update is one line such as `v=1 state=recording level=0.42 locked=0`, the same record walkie writes to its state file (the format is in the main README). walkie connects at the start of each dictation and sends a line every 40 ms while recording and every 250 ms otherwise.

- The plugin doesn't watch the state file: reloading it 25 times a second cost DMS about 7% of a core on a Ryzen 7 5700U, against 3% for reading the same records from the socket.
- The meter moves at most every 70 ms, showing the loudest level since it last moved. Every repaint costs DMS CPU, and 12 updates a second still look live.
- If a non-idle state goes 2 s without an update, the plugin assumes walkie died and hides the pill.
- The plugin reads format version 1. For any other version it shows nothing and logs one warning, so update walkie or the plugin to match. Unknown keys are ignored.

## Limits

DMS logs "Created graphical object was not placed in the graphics scene" when the plugin loads. Every daemon plugin gets it, because DMS creates the plugin's root item without a window, and it's harmless.
