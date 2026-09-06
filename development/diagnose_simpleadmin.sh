#!/bin/bash
set -u

section() { printf '\n=== %s ===\n' "$1"; }
section 'Platform / permissions'
date -u
uname -a
id
section 'Filesystems / space'
awk '$2 == "/" || $2 == "/usrdata" || $2 == "/tmp" {print}' /proc/mounts
df -Pk / /usrdata /tmp
section 'Application files (no credentials)'
for file in simpleadmin-httpd www/index.html www/login.html www/js/simpleadmin-spa.js run_simpleadmin.sh prepare_simpleadmin_ports.sh; do
    ls -ld "/usrdata/simpleadmin/$file" 2>/dev/null || true
done
if [ -x /usrdata/simpleadmin/simpleadmin-httpd ]; then
    /usrdata/simpleadmin/simpleadmin-httpd --version
fi
section 'AT device presence (not opened)'
ls -l /dev/smd11 2>/dev/null || true
section 'Service state'
systemctl show simpleadmin-httpd.service -p LoadState -p ActiveState -p SubState -p Result -p ExecMainStatus -p NRestarts -p FragmentPath 2>/dev/null || true
systemctl is-enabled simpleadmin-httpd.service 2>/dev/null || true
pidof simpleadmin-httpd 2>/dev/null || true
section 'IPv4 addresses / routes'
ip -4 addr show 2>/dev/null || ifconfig 2>/dev/null || true
ip -4 route show 2>/dev/null || true
section 'Configured HTTP port / listeners'
port=80
if [ -e /usrdata/simpleadmin/http_port ]; then port="$(cat /usrdata/simpleadmin/http_port)"; fi
if [[ "$port" =~ ^[1-9][0-9]{0,4}$ ]] && [ "$port" -le 65535 ]; then
    echo "HTTP_PORT=$port"
    port_hex="$(printf '%04X' "$port")"
    awk -v suffix=":$port_hex" 'FNR == 1 || (substr($2,length($2)-4) == suffix && $4 == "0A")' /proc/net/tcp /proc/net/tcp6 2>/dev/null || true
else
    echo 'Invalid HTTP port configuration; specify a valid port in the installer.'
fi
section 'IPv4 INPUT firewall'
iptables -L INPUT -n -v --line-numbers 2>/dev/null || true
section 'Loopback HTTP'
if [ -f /usrdata/simpleadmin/check_web.sh ]; then
    bash /usrdata/simpleadmin/check_web.sh || true
else
    echo 'Installed version has no check_web.sh; use the PC tunnel probe below.'
fi
section 'Recent startup / installation error (RAM only)'
for file in /tmp/simpleadmin-startup.log /tmp/simpleadmin-fallback-start.out /tmp/simpleadmin-install-result.env; do
    if [ -f "$file" ]; then
        echo "$file"
        tail -c 8192 "$file"
    fi
done
