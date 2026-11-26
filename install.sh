#!/usr/bin/env bash
# Build and install walkie, its model and the DMS plugin. Written for Fedora.
#
#   ./install.sh               # everything
#   SKIP_DNF=1 ./install.sh    # build deps already installed
#   ./install.sh unified       # also fetch Hex's larger model (731 MB)
#
# Puts:  ~/.local/bin/walkie
#        ~/.local/share/walkie/models/
#        ~/.config/dms/plugins/walkie/  (or ~/.config/DankMaterialShell/plugins/walkie/ on DMS 1.x)
set -euo pipefail
cd "$(dirname "$0")"

DATA=${XDG_DATA_HOME:-$HOME/.local/share}/walkie
CONF=${XDG_CONFIG_HOME:-$HOME/.config}
HF=https://huggingface.co/handy-computer

# 1. build dependencies (cargo from dnf only if there's no rustup toolchain)
if [ "${SKIP_DNF:-0}" != 1 ]; then
    pkgs=(gcc gcc-c++ cmake make git pkgconf-pkg-config pipewire-devel wl-clipboard libnotify)
    command -v cargo >/dev/null || pkgs+=(cargo rust)
    sudo dnf install -y "${pkgs[@]}"
fi

# 2. build. CARGO_BUILD_JOBS also caps the transcribe.cpp cmake build.
git submodule update --init
CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4} cargo build --release --locked
install -Dm755 target/release/walkie "$HOME/.local/bin/walkie"

# 3. model
get() { # get REPO FILE SHA256
    local out=$DATA/models/$2
    mkdir -p "$DATA/models"
    if [ ! -s "$out" ]; then
        curl -fL --retry 3 -o "$out.part" "$HF/$1-gguf/resolve/main/$2"
        mv "$out.part" "$out"
    fi
    if [ -n "$3" ]; then echo "$3  $out" | sha256sum -c --quiet; fi
}
get parakeet-tdt_ctc-110m parakeet-tdt_ctc-110m-Q8_0.gguf 7dd44c74a331d788a4e5f8b16913b3feb29ced22cf5613aad0e0f6cd30516296
case " $* " in *" unified "*) get parakeet-unified-en-0.6b parakeet-unified-en-0.6b-Q8_0.gguf "" ;; esac

# 4. DMS plugin: DMS 2.x reads ~/.config/dms, 1.x read ~/.config/DankMaterialShell
if [ -d "$CONF/dms" ] || [ ! -d "$CONF/DankMaterialShell" ]; then
    plugins=$CONF/dms/plugins
else
    plugins=$CONF/DankMaterialShell/plugins
fi
mkdir -p "$plugins"
rm -rf "$plugins/walkie"
cp -r plugin "$plugins/walkie"
if command -v dms >/dev/null; then
    dms ipc call plugin-scan scan >/dev/null 2>&1 || true
    dms ipc call plugins enable walkie >/dev/null 2>&1 &&
        echo "DMS plugin enabled" || echo "Enable Walkie under DMS Settings → Plugins"
fi

walkie --version
echo "Point kanata at: $HOME/.local/bin/walkie start (press) / $HOME/.local/bin/walkie stop (release)"
