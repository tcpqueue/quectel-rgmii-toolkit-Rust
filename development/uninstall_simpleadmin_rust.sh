#!/bin/bash

set -u

SYSTEMD_DIRS="/etc/systemd/system /lib/systemd/system"
SIMPLEADMIN_DIR="/usrdata/simpleadmin"
ROOT_BIN="/usrdata/root/bin"
POST_BOOT_FILE="/etc/init.post_boot.sh"
POST_BOOT_BEGIN="# BEGIN SIMPLEADMIN GO AUTOSTART"
POST_BOOT_END="# END SIMPLEADMIN GO AUTOSTART"

log() {
    echo "[信息] $*"
}

warn() {
    echo "[警告] $*"
}

remount_rw() {
    trap 'mount -o remount,ro / || echo "[错误] 根目录恢复只读失败" >&2' EXIT
    trap 'exit 1' INT TERM
    mount -o remount,rw / || exit 1
}

remount_ro() {
    sync
    mount -o remount,ro / || exit 1
    trap - EXIT INT TERM
}

stop_service() {
    systemctl stop "$1" >/dev/null 2>&1 || true
    systemctl disable "$1" >/dev/null 2>&1 || true
}

stop_fallback_process() {
    if [ -x "$SIMPLEADMIN_DIR/stop_simpleadmin.sh" ]; then
        "$SIMPLEADMIN_DIR/stop_simpleadmin.sh" >/dev/null 2>&1 || true
    fi
    if [ -f "$SIMPLEADMIN_DIR/simpleadmin-httpd.pid" ]; then
        local pid=""
        pid="$(cat "$SIMPLEADMIN_DIR/simpleadmin-httpd.pid" 2>/dev/null || true)"
        [ -n "$pid" ] && kill "$pid" >/dev/null 2>&1 || true
        rm -f "$SIMPLEADMIN_DIR/simpleadmin-httpd.pid"
    fi
    killall simpleadmin-httpd >/dev/null 2>&1 || true
    ps 2>/dev/null | awk '
        $0 ~ "cat /dev/ttyIN" { print $1 }
        $0 ~ "cat /dev/smd11 > /dev/ttyIN" { print $1 }
        $0 ~ "socat" && ($0 ~ "link=/dev/ttyIN" || $0 ~ "link=/dev/ttyOUT") { print $1 }
    ' | while read pid; do
        case "$pid" in
            ""|*[!0-9]*) continue ;;
        esac
        kill "$pid" >/dev/null 2>&1 || true
        sleep 1
        kill -9 "$pid" >/dev/null 2>&1 || true
    done
    rm -f /dev/ttyIN /dev/ttyOUT 2>/dev/null || true
}

remove_post_boot_autostart() {
    [ -f "$POST_BOOT_FILE" ] || return 0
    sed -i "/$POST_BOOT_BEGIN/,/$POST_BOOT_END/d" "$POST_BOOT_FILE" 2>/dev/null || true
}

remove_unit() {
    local unit="$1"
    local dir=""

    stop_service "$unit"
    for dir in $SYSTEMD_DIRS; do
        rm -f "$dir/$unit" "$dir/multi-user.target.wants/$unit" 2>/dev/null || true
    done
}

clear_go_ttl_rules() {
    if [ -x "$SIMPLEADMIN_DIR/simpleadmin-httpd" ]; then
        SIMPLEADMIN_MANAGE_ROOTFS=0 "$SIMPLEADMIN_DIR/simpleadmin-httpd" ttl off >/dev/null 2>&1 || warn "清理 Rust 原生 TTL 规则失败"
    fi
}

remove_simpleadmin_go_files() {
    log "正在卸载 SimpleAdmin Rust 服务和当前版本文件"
    clear_go_ttl_rules
    stop_fallback_process
    remove_post_boot_autostart

    remove_unit simpleadmin-httpd.service
    rm -f "$ROOT_BIN/simplepasswd"
    rm -f "$SIMPLEADMIN_DIR/simpleadmin-httpd"
    rm -f "$SIMPLEADMIN_DIR/simpleadmin.auth"
    rm -f "$SIMPLEADMIN_DIR/server.crt"
    rm -f "$SIMPLEADMIN_DIR/server.key"
    rm -f "$SIMPLEADMIN_DIR/zbims-ca.crt"
    rm -f "$SIMPLEADMIN_DIR/zbims-ca.key"
    rm -f "$SIMPLEADMIN_DIR/at_devices.conf"
    rm -f "$SIMPLEADMIN_DIR/ttlvalue"
    rm -f "$SIMPLEADMIN_DIR/monitor.json"
    rm -f /tmp/simpleadmin-httpd.pid
    rm -f "$SIMPLEADMIN_DIR/bridge0_mac"
    rm -f "$SIMPLEADMIN_DIR/mobileap_bridge0_mac.sh"
    rm -f "$SIMPLEADMIN_DIR/socat-armel-static"
    rm -f "$SIMPLEADMIN_DIR/simpleadmin-httpd.pid"
    rm -f "$SIMPLEADMIN_DIR/start_simpleadmin.sh"
    rm -f "$SIMPLEADMIN_DIR/stop_simpleadmin.sh"
    rm -f "$SIMPLEADMIN_DIR/check_web.sh" "$SIMPLEADMIN_DIR/run_simpleadmin.sh"
    rm -f "$SIMPLEADMIN_DIR/prepare_simpleadmin_ports.sh" "$SIMPLEADMIN_DIR/simpleadmin-httpd.new"
    rm -rf "$SIMPLEADMIN_DIR/www.new"
    rm -rf "$SIMPLEADMIN_DIR/www"
    rm -rf "$SIMPLEADMIN_DIR/systemd"

    rmdir "$SIMPLEADMIN_DIR" >/dev/null 2>&1 || true
}

reload_systemd() {
    log "正在重载 systemd 状态"
    systemctl daemon-reload >/dev/null 2>&1 || true
    systemctl reset-failed >/dev/null 2>&1 || true
}

main() {
    log "开始卸载 SimpleAdmin Rust 二进制版本"
    remount_rw
    remove_simpleadmin_go_files
    reload_systemd
    remount_ro
    log "卸载完成"
}

main "$@"
