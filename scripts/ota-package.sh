#!/usr/bin/env bash
# Builds the online update files for a release: the module package (the tracked development/
# tree as a gzip ustar archive), simpleadmin-ota.json and its Ed25519 signature.
set -euo pipefail
cd "$(dirname "$0")/.."
key="${SIMPLEADMIN_OTA_KEY:-$HOME/.config/simpleadmin/ota-ed25519.pem}"
[ -f "$key" ] || { echo "Signing key not found: $key (set SIMPLEADMIN_OTA_KEY)" >&2; exit 1; }
# Signing with any other key would publish an update no installed module accepts.
public="$(openssl pkey -in "$key" -pubout -outform DER | tail -c 32 | base64 -w0)"
[ "$public" = "$(tr -d '\n' < src/ota-public-key.txt)" ] || { echo "Signing key does not match src/ota-public-key.txt" >&2; exit 1; }
git diff --quiet HEAD -- development || { echo "Commit development/ before packaging." >&2; exit 1; }
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
package="simpleadmin-ota-$version.tar.gz"
mkdir -p packages
git ls-files -z development | LC_ALL=C sort -z |
    tar --null -T - --format=ustar --owner=0 --group=0 --numeric-owner --mtime=@0 -czf "packages/$package"
size="$(stat -c %s "packages/$package")"
sha="$(sha256sum "packages/$package" | cut -d' ' -f1)"
printf '{"version":"%s","tag":"v%s","package":"%s","size":%s,"sha256":"%s","published":"%s"}' \
    "$version" "$version" "$package" "$size" "$sha" "$(date -u +%Y-%m-%d)" > packages/simpleadmin-ota.json
openssl pkeyutl -sign -inkey "$key" -rawin -in packages/simpleadmin-ota.json -out packages/simpleadmin-ota.json.bin
base64 -w0 packages/simpleadmin-ota.json.bin > packages/simpleadmin-ota.json.sig
rm -f packages/simpleadmin-ota.json.bin
openssl pkeyutl -verify -pubin -inkey <(openssl pkey -in "$key" -pubout) -rawin \
    -in packages/simpleadmin-ota.json -sigfile <(base64 -d packages/simpleadmin-ota.json.sig) >/dev/null
echo "packages/$package ($size bytes), packages/simpleadmin-ota.json, packages/simpleadmin-ota.json.sig"
