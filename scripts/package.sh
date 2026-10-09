#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
case "$PWD" in /mnt/*) echo "Package in a Linux native directory." >&2; exit 1;; esac
sha256sum -c SHA256SUMS
(cd development && sha256sum -c SHA256SUMS)
test -f SimpleAdmin-Setup.exe || { echo "Run scripts/build.sh first." >&2; exit 1; }
mkdir -p packages
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
archive="packages/quectel-rgmii-toolkit-Rust-$version-offline.zip"
rm -f "$archive"
zip -q -j "$archive" SimpleAdmin-Setup.exe
cp SimpleAdmin-Setup.exe packages/SimpleAdmin-Setup.exe
(cd packages && sha256sum SimpleAdmin-Setup.exe "$(basename "$archive")" > SHA256SUMS-release.txt && cat SHA256SUMS-release.txt)
