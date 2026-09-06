#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../development"
case "$PWD" in /mnt/*) echo 'Generate checksums in a Linux native directory.' >&2; exit 1;; esac
find simpleadmin -type f -print0 | LC_ALL=C sort -z | xargs -0 sha256sum > SHA256SUMS
