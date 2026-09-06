#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
case "$PWD" in /mnt/*) echo "Build in a Linux native directory." >&2; exit 1;; esac
cargo test --locked
cargo build --locked --release --target armv7-unknown-linux-musleabihf
cargo build --locked --release --target x86_64-pc-windows-gnu
install -m 755 target/armv7-unknown-linux-musleabihf/release/simpleadmin-httpd development/simpleadmin/simpleadmin-httpd.armv7
install -d windows-test/bin
install -m 755 target/x86_64-pc-windows-gnu/release/simpleadmin-httpd.exe windows-test/bin/simpleadmin-httpd.exe
sha256sum development/simpleadmin/simpleadmin-httpd.armv7 windows-test/bin/simpleadmin-httpd.exe SimpleAdmin-Setup.exe > SHA256SUMS
bash scripts/checksums.sh
