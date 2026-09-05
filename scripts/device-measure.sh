#!/bin/sh
set -eu
label="${1:-runtime}"
ticks() {
    for pid in $(pidof simpleadmin-httpd 2>/dev/null); do
        [ -r "/proc/$pid/stat" ] || continue
        awk '{print $14+$15}' "/proc/$pid/stat"
    done | awk '{sum+=$1} END {print sum+0}'
}
total() { awk '/^cpu / {for(i=2;i<=9;i++)sum+=$i; print sum; exit}' /proc/stat; }
before="$(ticks)"
system_before="$(total)"
sleep 60
after="$(ticks)"
system_after="$(total)"
echo "label=$label"
echo "sample_seconds=60"
awk -v a="$before" -v b="$after" -v c="$system_before" -v d="$system_after" 'BEGIN {if(d>c) printf "cpu_percent=%.3f\n",100*(b-a)/(d-c)}'
for pid in $(pidof simpleadmin-httpd 2>/dev/null); do
    echo "pid=$pid"
    awk '/^(VmRSS|RssAnon|RssFile|Threads):/ {print}' "/proc/$pid/status"
    if [ -r "/proc/$pid/smaps_rollup" ]; then
        awk '/^Pss:/ {print}' "/proc/$pid/smaps_rollup"
    fi
done
stat -c 'binary_bytes=%s' "${2:-/usrdata/simpleadmin/simpleadmin-httpd}"
awk '$2=="/" {print "root_mount=" $4}' /proc/mounts
