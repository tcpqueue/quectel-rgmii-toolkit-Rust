#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
case "$PWD" in /mnt/*) echo "Run from a Linux native directory." >&2; exit 1;; esac
export RUST_FIXTURE_ROOT="$PWD"
source_dir="$(realpath "${1:?Pass the Go repository directory}")/go-build/simpleadmin-go"
scratch="$(mktemp -d "${TMPDIR:-/tmp}/simpleadmin-go-reference.XXXXXX")"
trap 'rm -rf "$scratch"' EXIT
cp -a "$source_dir/." "$scratch/"
cp tests/reference/export_rust_test.go "$scratch/cmd/simpleadmin-httpd/"
sed -i '/^func parseSMSListAT(raw string) map\[string\]any {$/a\    captureRustSMS(raw)' "$scratch/cmd/simpleadmin-httpd/structured_api.go"
cd "$scratch"
go test ./cmd/simpleadmin-httpd -run TestExportRustFixtures -count=1
