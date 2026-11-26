#!/bin/sh
# Build the portable release binary and package it.
#
#   scripts/dist.sh [VERSION]
#
# VERSION defaults to Cargo.toml's and must match it when given. Writes
# dist/walkie-VERSION-x86_64-linux.tar.gz and dist/SHA256SUMS. The release
# workflow runs this in a fedora:43 container. The binary needs glibc 2.39, so
# it also runs on Ubuntu 24.04, Debian 13 and Arch.
set -eu
cd "$(dirname "$0")/.."

crate=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
version=${1:-$crate}
if [ "$version" != "$crate" ]; then
    echo "version $version doesn't match Cargo.toml's $crate" >&2
    exit 1
fi
name=walkie-$version-x86_64-linux
# Before the build, so a git that can't read the repo fails in seconds.
epoch=${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct)}

# ggml for any x86-64 with AVX2, FMA, F16C and BMI2, not for the build machine.
WALKIE_GGML_NATIVE=0 CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4} cargo build --release --locked
bin=target/release/walkie
# A zmm register means AVX-512 code, which would crash on CPUs without it.
# The disassembly goes to a file so a failing objdump stops the script
# instead of handing grep nothing to find.
command -v objdump >/dev/null || { echo "needs objdump (binutils)" >&2; exit 1; }
objdump -d --no-show-raw-insn "$bin" >target/walkie.dis
if grep -q '%zmm' target/walkie.dis; then
    echo "$bin uses AVX-512 instructions; it must run on AVX2-only CPUs" >&2
    exit 1
fi

# The beeps are compiled into the binary, so sounds/ isn't shipped.
rm -rf dist
mkdir -p "dist/$name"
cp "$bin" "dist/$name/walkie"
cp -r plugin install.sh README.md LICENSE "dist/$name/"
tar -C dist --sort=name --owner=0 --group=0 --numeric-owner --mtime="@$epoch" -cf - "$name" |
    gzip -9n >"dist/$name.tar.gz"
rm -rf "dist/${name:?}"
(cd dist && sha256sum "$name.tar.gz" >SHA256SUMS)
cat dist/SHA256SUMS
