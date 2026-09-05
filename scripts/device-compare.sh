#!/bin/sh
set -eu
go_binary=/tmp/simpleadmin-compare/go/simpleadmin-httpd
rust_binary=/usrdata/simpleadmin/simpleadmin-httpd
[ -x "$go_binary" ]
[ -x "$rust_binary" ]
stop_test() {
    for pid in $(pidof simpleadmin-httpd 2>/dev/null); do
        executable="$(readlink "/proc/$pid/exe" 2>/dev/null || true)"
        case "$executable" in
            "$go_binary"|"$rust_binary") kill "$pid" 2>/dev/null || true ;;
        esac
    done
    sleep 1
}
restore() {
    stop_test
    systemctl start simpleadmin-httpd.service
}
trap restore EXIT
trap 'exit 1' INT TERM
systemctl stop simpleadmin-httpd.service
for kind in go rust; do
    stop_test
    if [ "$kind" = go ]; then binary="$go_binary"; else binary="$rust_binary"; fi
    "$binary" -http 127.0.0.1:18090 -no-tls -static /usrdata/simpleadmin/www \
        -auth-file /usrdata/simpleadmin/simpleadmin.auth \
        -ttl-file /usrdata/simpleadmin/ttlvalue > /dev/null 2>&1 &
    runtime_pid=$!
    sleep 30
    kill -0 "$runtime_pid"
    sh /tmp/device-measure.sh "$kind-isolated" "$binary"
done
