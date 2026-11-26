#!/bin/sh
# Install walkie, its model and the DMS plugin. Written for Fedora on x86_64.
#
#   curl -fsSL https://raw.githubusercontent.com/BalajiLeninrajan/walkie/main/install.sh | sh
#   ./install.sh                             # the latest release
#   WALKIE_VERSION=v0.2.0 ./install.sh       # a given release
#   ./install.sh unified                     # also fetch Hex's larger model (731 MB)
#   ./install.sh --from-source               # build this checkout (installs build deps with sudo dnf)
#   SKIP_DNF=1 ./install.sh --from-source    # build deps already installed
#
# Through curl, pass arguments after `sh -s --`. Run from an unpacked release
# tarball, it installs that tarball's files instead of downloading.
#
# Puts:  ~/.local/bin/walkie
#        ~/.local/share/walkie/models/
#        ~/.config/dms/plugins/walkie/  (or ~/.config/DankMaterialShell/plugins/walkie/ on DMS 1.x)
set -eu

# Everything runs from main, which sh reaches only after reading the last line,
# so a download cut off halfway runs nothing.
main() {
    REPO=BalajiLeninrajan/walkie
    BIN=$HOME/.local/bin
    DATA=${XDG_DATA_HOME:-$HOME/.local/share}/walkie
    CONF=${XDG_CONFIG_HOME:-$HOME/.config}
    HF=https://huggingface.co/handy-computer

    die() {
        echo "install.sh: $*" >&2
        exit 1
    }

    from_source=0
    unified=0
    for arg in "$@"; do
        case $arg in
            --from-source) from_source=1 ;;
            unified) unified=1 ;;
            -h | --help)
                echo "usage: install.sh [--from-source] [unified]   (WALKIE_VERSION=vX.Y.Z picks a release)"
                exit 0
                ;;
            *) die "unknown argument: $arg" ;;
        esac
    done

    # Piped into sh, $0 is the shell and there is no checkout to look at.
    here=
    case $0 in
        install.sh | */install.sh) [ -f "$0" ] && here=$(cd "$(dirname "$0")" && pwd) ;;
    esac
    checkout=0
    [ -n "$here" ] && [ -f "$here/Cargo.toml" ] && [ -f "$here/build.rs" ] && checkout=1

    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    trap 'exit 1' HUP INT TERM

    # Sets $src to an unpacked release: walkie, plugin/, README.md...
    fetch_release() {
        tag=${WALKIE_VERSION:-}
        if [ -z "$tag" ]; then
            # github.com redirects this to .../releases/tag/vX.Y.Z. The API would
            # say the same, but it allows 60 requests an hour per IP.
            url=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest") ||
                die "couldn't find the latest release of $REPO; set WALKIE_VERSION=vX.Y.Z to pick one"
            tag=${url##*/}
            case $tag in
                v*) ;;
                *) die "couldn't find the latest release of $REPO; set WALKIE_VERSION=vX.Y.Z to pick one" ;;
            esac
        fi
        case $tag in v*) ;; *) tag=v$tag ;; esac
        name=walkie-${tag#v}-x86_64-linux
        # WALKIE_RELEASE_URL serves the same files from elsewhere (tests use file://).
        base=${WALKIE_RELEASE_URL:-https://github.com/$REPO/releases/download/$tag}
        echo "Downloading walkie $tag"
        curl -fL --retry 3 -o "$tmp/$name.tar.gz" "$base/$name.tar.gz"
        curl -fsSL --retry 3 -o "$tmp/SHA256SUMS" "$base/SHA256SUMS"
        awk -v f="$name.tar.gz" '$2 == f || $2 == "*" f' "$tmp/SHA256SUMS" >"$tmp/want"
        [ -s "$tmp/want" ] || die "SHA256SUMS has no line for $name.tar.gz"
        (cd "$tmp" && sha256sum -c --quiet want) || die "checksum mismatch for $name.tar.gz"
        tar -xzf "$tmp/$name.tar.gz" -C "$tmp"
        src=$tmp/$name
        if [ ! -x "$src/walkie" ] || [ ! -f "$src/plugin/plugin.json" ]; then
            die "$name.tar.gz is missing walkie or plugin/"
        fi
    }

    # Builds target/release/walkie in the checkout and sets $src to the checkout.
    build_from_source() {
        [ "$checkout" = 1 ] ||
            die "--from-source builds a checkout: git clone --recurse-submodules https://github.com/$REPO"
        cd "$here"
        # cargo from dnf only if there's no rustup toolchain
        if [ "${SKIP_DNF:-0}" != 1 ]; then
            pkgs="gcc gcc-c++ cmake make git pkgconf-pkg-config pipewire-devel wl-clipboard libnotify"
            command -v cargo >/dev/null || pkgs="$pkgs cargo rust"
            # shellcheck disable=SC2086 # one word per package
            sudo dnf install -y $pkgs
        fi
        # CARGO_BUILD_JOBS also caps the transcribe.cpp cmake build.
        git submodule update --init
        CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4} cargo build --release --locked
        src=$here
    }

    # 1. the binary
    if [ "$from_source" = 1 ]; then
        build_from_source
        bin_src=$src/target/release/walkie
    else
        [ "$(uname -m)" = x86_64 ] || die "releases are x86_64 only; build a checkout with --from-source"
        # Release builds use these. Without one, walkie dies with SIGILL at the
        # first dictation, long after --version said all was well.
        for f in sse4_2 avx2 fma f16c bmi2; do
            grep -qw "$f" /proc/cpuinfo || die "this CPU lacks $f, which releases need; build a checkout with --from-source"
        done
        for c in curl tar gzip sha256sum awk; do
            command -v "$c" >/dev/null || die "needs $c"
        done
        if [ -n "$here" ] && [ "$checkout" = 0 ] && [ -x "$here/walkie" ] && [ -f "$here/plugin/plugin.json" ]; then
            echo "Installing the unpacked release in $here"
            src=$here
        else
            [ "$checkout" = 1 ] && echo "Installing a release. ./install.sh --from-source builds this checkout instead."
            fetch_release
        fi
        bin_src=$src/walkie
    fi

    # 2. model, before anything is replaced: a failed download leaves the old install as it was
    get() { # get REPO FILE SHA256
        out=$DATA/models/$2
        mkdir -p "$DATA/models"
        if [ -s "$out" ] && { [ -z "$3" ] || echo "$3  $out" | sha256sum -c --quiet 2>/dev/null; }; then
            return 0
        fi
        curl -fL --retry 3 -o "$out.part" "$HF/$1-gguf/resolve/main/$2"
        if [ -n "$3" ] && ! echo "$3  $out.part" | sha256sum -c --quiet; then
            rm -f "$out.part"
            die "checksum mismatch for $2"
        fi
        mv "$out.part" "$out"
    }
    get parakeet-tdt_ctc-110m parakeet-tdt_ctc-110m-Q8_0.gguf 7dd44c74a331d788a4e5f8b16913b3feb29ced22cf5613aad0e0f6cd30516296
    [ "$unified" = 1 ] && get parakeet-unified-en-0.6b parakeet-unified-en-0.6b-Q8_0.gguf ""

    # 3. the binary and the DMS plugin, together.
    # DMS 2.x reads ~/.config/dms, 1.x read ~/.config/DankMaterialShell
    if [ -d "$CONF/dms" ] || [ ! -d "$CONF/DankMaterialShell" ]; then
        plugins=$CONF/dms/plugins
    else
        plugins=$CONF/DankMaterialShell/plugins
    fi
    [ -f "$src/plugin/plugin.json" ] || die "$src/plugin is missing"
    install -Dm755 "$bin_src" "$BIN/walkie"
    mkdir -p "$plugins"
    rm -rf "$plugins/walkie"
    cp -r "$src/plugin" "$plugins/walkie"
    if command -v dms >/dev/null; then
        dms ipc call plugin-scan scan </dev/null >/dev/null 2>&1 || true
        # dms ipc exits 0 even when the call fails, and the scan above is
        # debounced, so an enable sent right after it answers
        # PLUGIN_NOT_FOUND. Read the reply and retry for up to 5 s.
        enabled=
        tries=0
        while [ "$tries" -lt 10 ]; do
            case $(dms ipc call plugins enable walkie </dev/null 2>/dev/null) in
                PLUGIN_ENABLE_SUCCESS*) enabled=1; break ;;
            esac
            tries=$((tries + 1))
            sleep 0.5
        done
        if [ -n "$enabled" ]; then
            echo "DMS plugin enabled"
        else
            echo "Enable Walkie under DMS Settings → Plugins"
        fi
    fi

    # 4. runtime dependencies. Checked, never installed: that would need sudo.
    has_lib() {
        ldconfig -p 2>/dev/null | grep -qF "$1" || [ -e "/usr/lib64/$1" ] || [ -e "/lib64/$1" ]
    }
    missing=
    has_lib libpipewire-0.3.so.0 || missing="$missing pipewire-libs"
    has_lib libstdc++.so.6 || missing="$missing libstdc++"
    command -v notify-send >/dev/null || missing="$missing libnotify"
    command -v wl-copy >/dev/null || missing="$missing wl-clipboard"
    if [ -n "$missing" ]; then
        echo "walkie is installed, but it needs these packages. Install them with:"
        echo "  sudo dnf install$missing"
        case $missing in *pipewire-libs* | *libstdc++*) exit 1 ;; esac
    fi

    "$BIN/walkie" --version
    echo "Point kanata at: $BIN/walkie start (press) / $BIN/walkie stop (release)"
}

main "$@"
