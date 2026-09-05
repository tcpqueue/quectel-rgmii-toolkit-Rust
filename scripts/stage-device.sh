#!/bin/sh
set -eu
binary=/tmp/simpleadmin-rust-test/simpleadmin-httpd
chmod 777 "$binary"
"$binary" --version
systemctl stop simpleadmin-httpd.service
if pidof simpleadmin-httpd >/dev/null 2>&1; then
    echo 'An AT reader is still active; restore the existing service.' >&2
    systemctl start simpleadmin-httpd.service
    exit 1
fi
SIMPLEADMIN_MANAGE_ROOTFS=1 nohup "$binary" --http :80 --static /usrdata/simpleadmin/www --auth-file /usrdata/simpleadmin/simpleadmin.auth --ttl-file /usrdata/simpleadmin/ttlvalue > /tmp/simpleadmin-rust-test/start.log 2>&1 < /dev/null &
runtime_pid=$!
echo "$runtime_pid" > /tmp/simpleadmin-rust-test/pid
sleep 2
if ! kill -0 "$runtime_pid" 2>/dev/null; then
    cat /tmp/simpleadmin-rust-test/start.log
    systemctl start simpleadmin-httpd.service
    exit 1
fi
echo "Rust staging process: $runtime_pid"
