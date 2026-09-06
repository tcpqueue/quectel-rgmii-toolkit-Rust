#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
case "$PWD" in /mnt/*) echo "Package in a Linux native directory." >&2; exit 1;; esac
sha256sum -c SHA256SUMS
(cd development && sha256sum -c SHA256SUMS)
mkdir -p packages
archive="packages/quectel-rgmii-toolkit-Rust-$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)-offline.zip"
rm -f "$archive"
zip -q "$archive" SimpleAdmin-Setup.exe
sha256sum "$archive"
