#!/bin/sh
set -eu
source=/tmp/simpleadmin-httpd-final
destination=/usrdata/simpleadmin/simpleadmin-httpd
[ -s "$source" ]
restore() {
    mount -o remount,ro / || echo 'Root read-only restoration failed.' >&2
    systemctl start simpleadmin-httpd.service
}
trap restore EXIT
trap 'exit 1' INT TERM
systemctl stop simpleadmin-httpd.service
mount -o remount,rw /
cp "$source" "$destination.new"
chmod 777 "$destination.new"
mv "$destination.new" "$destination"
sync
mount -o remount,ro /
systemctl start simpleadmin-httpd.service
systemctl is-active simpleadmin-httpd.service
trap - EXIT INT TERM
sha256sum "$destination"
