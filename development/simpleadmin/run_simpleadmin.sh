#!/bin/sh
set -eu
SIMPLEADMIN_DIR=/usrdata/simpleadmin
export SIMPLEADMIN_MANAGE_ROOTFS=1

# The server emits one startup line or a fatal error; truncate on every restart.
# Never redirect this output to NAND on firmware without a tmpfs /tmp.
if [ "$(stat -f -c %T /tmp 2>/dev/null)" = tmpfs ]; then
    umask 077
    exec > /tmp/simpleadmin-startup.log 2>&1
else
    exec > /dev/null 2>&1
fi
exec "$SIMPLEADMIN_DIR/simpleadmin-httpd" \
    -http :80 -no-tls \
    -static "$SIMPLEADMIN_DIR/www" \
    -auth-file "$SIMPLEADMIN_DIR/simpleadmin.auth" \
    -at-devices-file "$SIMPLEADMIN_DIR/at_devices.conf"
