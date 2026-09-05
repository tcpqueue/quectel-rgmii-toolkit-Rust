#!/bin/sh
set -eu
scratch="$(mktemp -d /tmp/simpleadmin-write-check.XXXXXX)"
trap 'rm -rf "$scratch"' EXIT
snapshot() {
    find /usrdata/simpleadmin -type f -exec stat -c '%n %s %Y' '{}' ';' | sort
    sha256sum /etc/shadow
}
snapshot > "$scratch/before"
sleep 310
snapshot > "$scratch/after"
cmp -s "$scratch/before" "$scratch/after"
echo 'Application files and root password unchanged during 310 seconds of background monitoring.'
awk '$2=="/" {print "root_mount=" $4}' /proc/mounts
