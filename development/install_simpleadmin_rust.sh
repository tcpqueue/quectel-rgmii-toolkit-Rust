#!/bin/bash

set -Eeuo pipefail

PKG_DIR="/tmp/development"
SIMPLEADMIN_SRC="$PKG_DIR/simpleadmin"

SIMPLEADMIN_DIR="/usrdata/simpleadmin"
TTL_VALUE_FILE="$SIMPLEADMIN_DIR/ttlvalue"
AT_DEVICES_FILE="$SIMPLEADMIN_DIR/at_devices.conf"
SYSTEMD_UNIT="simpleadmin-httpd.service"
SYSTEMD_DIR=""
WANTS_DIR=""
SERVICE_UNIT_INSTALLED="0"
FALLBACK_PID_FILE="/tmp/simpleadmin-httpd.pid"
FALLBACK_START_SCRIPT="$SIMPLEADMIN_DIR/start_simpleadmin.sh"
FALLBACK_STOP_SCRIPT="$SIMPLEADMIN_DIR/stop_simpleadmin.sh"
PORT_PREPARE_SCRIPT="$SIMPLEADMIN_DIR/prepare_simpleadmin_ports.sh"
POST_BOOT_FILE="/etc/init.post_boot.sh"
POST_BOOT_LOG_FILE="/dev/null"
REBOOT_MARKER_FILE="/tmp/simpleadmin-reboot-required"
INSTALL_RESULT_FILE="/tmp/simpleadmin-install-result.env"
POST_BOOT_BEGIN="# BEGIN SIMPLEADMIN RUST AUTOSTART"
POST_BOOT_END="# END SIMPLEADMIN RUST AUTOSTART"
ROOT_BIN="/usrdata/root/bin"
MOBILEAP_HELPER_SRC="$SIMPLEADMIN_SRC/mobileap_bridge0_mac.sh"
MOBILEAP_HELPER_SCRIPT="$SIMPLEADMIN_DIR/mobileap_bridge0_mac.sh"
MOBILEAP_RESULT_FILE="/tmp/simpleadmin-mobileap-result.env"
MOBILEAP_CFG_TOUCHED="0"

log() {
    echo "[信息] $*"
}

warn() {
    echo "[警告] $*"
}

write_install_failure_result() {
    local msg="$1"
    {
        echo "INSTALL_STATUS=FAIL"
        echo "REBOOT_REQUIRED=0"
        echo "INSTALL_ERROR=$msg"
    } > "$INSTALL_RESULT_FILE" 2>/dev/null || true
}

write_install_success_result() {
    if [ -f "$INSTALL_RESULT_FILE" ]; then
        grep -q '^INSTALL_STATUS=' "$INSTALL_RESULT_FILE" 2>/dev/null || echo "INSTALL_STATUS=OK" >> "$INSTALL_RESULT_FILE"
    else
        {
            echo "INSTALL_STATUS=OK"
            echo "REBOOT_REQUIRED=0"
        } > "$INSTALL_RESULT_FILE" 2>/dev/null || true
    fi
}

fail() {
    write_install_failure_result "$*"
    echo "[错误] $*" >&2
    exit 1
}

trap 'fail "安装命令失败（第 $LINENO 行），请查看诊断报告"' ERR

preflight() {
    [ "$(id -u)" = "0" ] || fail "ADB shell 不是 root，无法安装"
    require_file "$PKG_DIR/SHA256SUMS"
    require_file "$SIMPLEADMIN_SRC/www/index.html"
    require_file "$SIMPLEADMIN_SRC/www/login.html"
    (cd "$PKG_DIR" && sha256sum -c SHA256SUMS) || fail "安装包校验失败，请重新完整解压并上传"
    chmod +x "$SIMPLEADMIN_SRC/simpleadmin-httpd.armv7"
    "$SIMPLEADMIN_SRC/simpleadmin-httpd.armv7" --version || fail "程序不能在此固件执行（架构、内核或执行权限不兼容）"
    local needed available
    needed="$(du -sk "$SIMPLEADMIN_SRC" | awk '{print $1}')"
    available="$(df -Pk /usrdata | awk 'END {print $4}')"
    case "$needed:$available" in *[!0-9:]*|:*|*:) fail "无法检查 /usrdata 剩余空间" ;; esac
    [ "$available" -gt "$((needed + 2048))" ] || fail "/usrdata 空间不足，需要安装包大小加 2 MiB 余量"
}

wait_for_web() {
    local attempt=0
    while [ "$attempt" -lt 10 ]; do
        attempt=$((attempt + 1))
        if bash "$SIMPLEADMIN_DIR/check_web.sh" >/dev/null 2>&1; then
            sleep 1
            bash "$SIMPLEADMIN_DIR/check_web.sh" >/dev/null 2>&1 && return 0
        fi
        sleep 1
    done
    bash "$SIMPLEADMIN_DIR/check_web.sh" || true
    return 1
}

remount_rw() {
    trap 'sync; mount -o remount,ro / || echo "[错误] 根目录恢复只读失败" >&2' EXIT
    trap 'exit 1' INT TERM
    mount -o remount,rw / || fail "根目录切换为读写失败"
}

remount_ro() {
    sync
    mount -o remount,ro / || fail "根目录恢复只读失败"
    trap - EXIT INT TERM
}

require_file() {
    [ -f "$1" ] || fail "缺少文件: $1"
}

is_valid_auth_file() {
    local file="$1"
    local line=""

    [ -s "$file" ] || return 1
    line="$(grep -v '^[[:space:]]*#' "$file" 2>/dev/null | grep -v '^[[:space:]]*$' | head -n 1 || true)"
    case "$line" in
        *:*)
            [ -n "${line%%:*}" ] || return 1
            [ -n "${line#*:}" ] || return 1
            return 0
            ;;
    esac
    return 1
}

write_default_auth_file() {
    umask 077
    printf 'admin:admin\n' > "$SIMPLEADMIN_DIR/simpleadmin.auth"
    chmod 600 "$SIMPLEADMIN_DIR/simpleadmin.auth"
}

is_writable_dir() {
    local dir="$1"
    [ -d "$dir" ] || mkdir -p "$dir" 2>/dev/null || return 1
    [ -w "$dir" ]
}

find_systemd_dir() {
    local dir="/lib/systemd/system"

    if is_writable_dir "$dir"; then
        echo "$dir"
        return 0
    fi

    return 1
}

remove_unit_files_from_dir() {
    local dir="$1"
    local unit="$2"

    [ -n "$dir" ] || return 0
    rm -f "$dir/$unit" "$dir/multi-user.target.wants/$unit" 2>/dev/null || true
}

remove_systemd_unit_files() {
    remove_unit_files_from_dir /etc/systemd/system "$SYSTEMD_UNIT"
    remove_unit_files_from_dir /lib/systemd/system "$SYSTEMD_UNIT"
}

remove_stale_etc_unit() {
    remove_unit_files_from_dir /etc/systemd/system "$SYSTEMD_UNIT"
}

link_unit() {
    local unit="$1"

    [ -n "$SYSTEMD_DIR" ] || return 1
    [ -n "$WANTS_DIR" ] || return 1
    mkdir -p "$WANTS_DIR" 2>/dev/null || return 1
    ln -sf "$SYSTEMD_DIR/$unit" "$WANTS_DIR/$unit"
}

remove_post_boot_autostart() {
    [ -f "$POST_BOOT_FILE" ] || return 0
    sed -i "/$POST_BOOT_BEGIN/,/$POST_BOOT_END/d" "$POST_BOOT_FILE" 2>/dev/null || true
    # Remove the old block on upgrade, without creating it again.
    sed -i '/# BEGIN SIMPLEADMIN GO AUTOSTART/,/# END SIMPLEADMIN GO AUTOSTART/d' "$POST_BOOT_FILE" 2>/dev/null || true
}

install_post_boot_autostart() {
    local parent=""

    parent="$(dirname "$POST_BOOT_FILE")"
    if [ ! -d "$parent" ] || [ ! -w "$parent" ]; then
        warn "post_boot 目录不可写，未安装自启动: $parent"
        return 1
    fi

    if [ ! -f "$POST_BOOT_FILE" ]; then
        warn "未找到 post_boot 脚本，跳过自启动安装: $POST_BOOT_FILE"
        return 1
    fi

    if [ ! -w "$POST_BOOT_FILE" ]; then
        warn "post_boot 脚本不可写，未安装自启动: $POST_BOOT_FILE"
        return 1
    fi

    remove_post_boot_autostart
    cat >> "$POST_BOOT_FILE" <<EOF
$POST_BOOT_BEGIN
(
    sleep 3
    if [ -x "$FALLBACK_START_SCRIPT" ]; then
        if command -v iptables >/dev/null 2>&1; then
            iptables -C INPUT -p tcp --dport 80 -j ACCEPT >/dev/null 2>&1 || iptables -I INPUT -p tcp --dport 80 -j ACCEPT >/dev/null 2>&1 || true
        fi
        "$FALLBACK_START_SCRIPT" >> "$POST_BOOT_LOG_FILE" 2>&1
    fi
) &
$POST_BOOT_END
EOF
    [ "$?" = 0 ] || return 1
    chmod +x "$POST_BOOT_FILE" 2>/dev/null || true
    log "post_boot 自启动已安装: $POST_BOOT_FILE"
}

install_at_device_config() {
    mkdir -p "$SIMPLEADMIN_DIR"
    log "正在写入 AT 设备配置：直接使用 /dev/smd11"
    cat > "$AT_DEVICES_FILE" <<'EOF'
/dev/smd11
EOF
    chmod 0644 "$AT_DEVICES_FILE"
}

stop_existing_simpleadmin_runtime() {
    log "正在停止旧的 SimpleAdmin 运行实例"
    log "正在请求 systemd 停止应用服务"
    systemctl stop "$SYSTEMD_UNIT" >/dev/null 2>&1 || true
    log "正在停用旧服务自启动"
    systemctl disable "$SYSTEMD_UNIT" >/dev/null 2>&1 || true
    # Never execute an old stop script or trust its stale PID file.
    . "$SIMPLEADMIN_SRC/runtime_processes.sh"
    log "正在按可执行文件路径核对残留进程"
    stop_runtime_kind server
    cleanup_legacy_at_bridges
    rm -f "$FALLBACK_PID_FILE"
    log "旧应用进程清理完成"
    remove_systemd_unit_files
    systemctl daemon-reload >/dev/null 2>&1 || true
    systemctl reset-failed >/dev/null 2>&1 || true
}

install_ttl_state() {
    log "正在初始化 Rust 原生 TTL 配置"
    local ttl_value="0"

    if [ -f "$TTL_VALUE_FILE" ]; then
        ttl_value="$(grep -o "[0-9]\{1,3\}" "$TTL_VALUE_FILE" 2>/dev/null | head -n 1 || true)"
    fi

    case "$ttl_value" in
        ""|*[!0-9]*) ttl_value="0" ;;
    esac
    if [ "$ttl_value" -gt 255 ]; then
        ttl_value="0"
    fi

    mkdir -p "$SIMPLEADMIN_DIR"
    echo "$ttl_value" > "$TTL_VALUE_FILE"
    chmod 0644 "$TTL_VALUE_FILE"
}

install_mobileap_helper_script() {
    require_file "$MOBILEAP_HELPER_SRC"
    cp -f "$MOBILEAP_HELPER_SRC" "$MOBILEAP_HELPER_SCRIPT"
    chmod +x "$MOBILEAP_HELPER_SCRIPT"
}

maybe_install_bridge0_mac_config() {
    MOBILEAP_CFG_TOUCHED="0"
    rm -f "$MOBILEAP_RESULT_FILE" 2>/dev/null || true

    if [ ! -x "$MOBILEAP_HELPER_SCRIPT" ]; then
        warn "mobileap bridge0 MAC 辅助脚本缺失或不可执行: $MOBILEAP_HELPER_SCRIPT"
        return 0
    fi

    SIMPLEADMIN_DIR="$SIMPLEADMIN_DIR" \
    MOBILEAP_RESULT_FILE="$MOBILEAP_RESULT_FILE" \
    SIMPLEADMIN_FIX_BRIDGE0_MAC="${SIMPLEADMIN_FIX_BRIDGE0_MAC:-1}" \
    "$MOBILEAP_HELPER_SCRIPT" || warn "mobileap bridge0 MAC 辅助脚本执行失败"

    if [ -f "$MOBILEAP_RESULT_FILE" ]; then
        # shellcheck disable=SC1090
        . "$MOBILEAP_RESULT_FILE" 2>/dev/null || true
    fi

    case "${MOBILEAP_CFG_TOUCHED:-0}" in
        1) MOBILEAP_CFG_TOUCHED="1" ;;
        *) MOBILEAP_CFG_TOUCHED="0" ;;
    esac
}

install_simpleadmin_files() {
    log "正在安装 SimpleAdmin Rust 运行文件"
    require_file "$SIMPLEADMIN_SRC/simpleadmin-httpd.armv7"
    require_file "$SIMPLEADMIN_SRC/systemd/simpleadmin-httpd.service"
    require_file "$SIMPLEADMIN_SRC/simplepasswd"
    require_file "$MOBILEAP_HELPER_SRC"

    mkdir -p "$SIMPLEADMIN_DIR" "$SIMPLEADMIN_DIR/www" "$ROOT_BIN"

    if [ -f "$SIMPLEADMIN_DIR/simpleadmin.auth" ] && ! is_valid_auth_file "$SIMPLEADMIN_DIR/simpleadmin.auth"; then
        warn "现有认证文件为空或格式无效，已重置为默认 admin/admin"
    fi

    # Finish and verify both copies before stopping the installed service.
    rm -rf "$SIMPLEADMIN_DIR/www.new"
    cp -f "$SIMPLEADMIN_SRC/simpleadmin-httpd.armv7" "$SIMPLEADMIN_DIR/simpleadmin-httpd.new"
    cp -r "$SIMPLEADMIN_SRC/www" "$SIMPLEADMIN_DIR/www.new"
    cmp "$SIMPLEADMIN_SRC/simpleadmin-httpd.armv7" "$SIMPLEADMIN_DIR/simpleadmin-httpd.new"
    (cd "$SIMPLEADMIN_DIR/www.new" && sed -n 's#  simpleadmin/www/#  #p' "$PKG_DIR/SHA256SUMS" | sha256sum -c -)
    stop_existing_simpleadmin_runtime
    rm -rf "$SIMPLEADMIN_DIR/www" "$SIMPLEADMIN_DIR/console" "$SIMPLEADMIN_DIR/systemd"
    mkdir -p "$SIMPLEADMIN_DIR/systemd"

    mv -f "$SIMPLEADMIN_DIR/simpleadmin-httpd.new" "$SIMPLEADMIN_DIR/simpleadmin-httpd"
    chmod 777 "$SIMPLEADMIN_DIR" "$SIMPLEADMIN_DIR/simpleadmin-httpd"

    mv "$SIMPLEADMIN_DIR/www.new" "$SIMPLEADMIN_DIR/www"
    cp -f "$SIMPLEADMIN_SRC/check_web.sh" "$SIMPLEADMIN_SRC/run_simpleadmin.sh" "$SIMPLEADMIN_DIR/"
    chmod 755 "$SIMPLEADMIN_DIR/check_web.sh" "$SIMPLEADMIN_DIR/run_simpleadmin.sh"

    cp -f "$SIMPLEADMIN_SRC/simplepasswd" "$ROOT_BIN/simplepasswd"
    chmod +x "$ROOT_BIN/simplepasswd"

    install_mobileap_helper_script

    if ! is_valid_auth_file "$SIMPLEADMIN_DIR/simpleadmin.auth"; then
        write_default_auth_file
    fi
    chmod 600 "$SIMPLEADMIN_DIR/simpleadmin.auth"

    cp -f "$SIMPLEADMIN_SRC/systemd/$SYSTEMD_UNIT" "$SIMPLEADMIN_DIR/systemd/$SYSTEMD_UNIT"
    install_fallback_scripts
    install_systemd_unit || warn "systemd 服务无法安装到 /lib/systemd/system；如果服务无法启动，将使用 post_boot 兜底"
}

install_fallback_scripts() {
    cp -f "$SIMPLEADMIN_SRC/runtime_processes.sh" "$SIMPLEADMIN_DIR/runtime_processes.sh"
    cat > "$PORT_PREPARE_SCRIPT" <<'EOF'
#!/bin/sh
. /usrdata/simpleadmin/runtime_processes.sh

open_web_port() {
    if command -v iptables >/dev/null 2>&1; then
        iptables -C INPUT -p tcp --dport 80 -j ACCEPT >/dev/null 2>&1 ||
            iptables -I INPUT 1 -p tcp --dport 80 -j ACCEPT || return 1
    else
        echo "[警告] 未找到 iptables，请检查固件防火墙是否允许 TCP 80" >&2
    fi
}

port80_listener_inodes() {
    awk 'FNR > 1 && $2 ~ /:0050$/ && $4 == "0A" {print $10}' /proc/net/tcp /proc/net/tcp6 2>/dev/null | sort -u
}
port80_owner_pids() {
    for inode in $(port80_listener_inodes); do
        for fd in /proc/[0-9]*/fd/*; do
            [ "$(readlink "$fd" 2>/dev/null)" = "socket:[$inode]" ] || continue
            pid="${fd#/proc/}"; echo "${pid%%/*}"
        done
    done | sort -u
}
stop_known_web_conflicts() {
    for pid in $(port80_owner_pids); do
        if [ "$(runtime_kind "$pid")" = server ]; then
            stop_runtime_kind server
        elif [ "$(readlink "/proc/$pid/exe" 2>/dev/null)" = /usr/sbin/lighttpd ] ||
             [ "$(readlink "/proc/$pid/exe" 2>/dev/null)" = /usr/bin/lighttpd ]; then
            if tr '\000' '\n' < "/proc/$pid/cmdline" | grep -qx /data/lighttpd.conf; then
                kill "$pid" 2>/dev/null || true
            fi
        fi
    done
}
describe_port80_owners() {
    for pid in $(port80_owner_pids); do
        echo "pid=$pid executable=$(readlink "/proc/$pid/exe" 2>/dev/null)"
    done
}
wait_for_port80_free() {
    for attempt in 1 2 3 4 5; do
        [ -z "$(port80_listener_inodes)" ] && return 0
        stop_known_web_conflicts
        sleep 1
    done
    [ -z "$(port80_listener_inodes)" ] && return 0
    echo "[错误] 80 端口仍被占用，SimpleAdmin Rust 无法绑定 :80" >&2
    describe_port80_owners >&2
    return 1
}
open_web_port || exit 1
stop_known_web_conflicts
wait_for_port80_free
EOF
    cat > "$FALLBACK_START_SCRIPT" <<'EOF'
#!/bin/sh
SIMPLEADMIN_DIR=/usrdata/simpleadmin
. "$SIMPLEADMIN_DIR/runtime_processes.sh"
if [ -n "$(runtime_pids server)" ]; then
    bash "$SIMPLEADMIN_DIR/check_web.sh"
    exit "$?"
fi
"$SIMPLEADMIN_DIR/prepare_simpleadmin_ports.sh" || exit 1
cleanup_legacy_at_bridges
nohup "$SIMPLEADMIN_DIR/run_simpleadmin.sh" >/dev/null 2>&1 < /dev/null &
sleep 2
bash "$SIMPLEADMIN_DIR/check_web.sh"
EOF
    cat > "$FALLBACK_STOP_SCRIPT" <<'EOF'
#!/bin/sh
. /usrdata/simpleadmin/runtime_processes.sh
stop_runtime_kind server
cleanup_legacy_at_bridges
rm -f /tmp/simpleadmin-httpd.pid
EOF
    chmod 755 "$FALLBACK_START_SCRIPT" "$FALLBACK_STOP_SCRIPT" "$PORT_PREPARE_SCRIPT" "$SIMPLEADMIN_DIR/runtime_processes.sh"
}

install_systemd_unit() {
    SYSTEMD_DIR="$(find_systemd_dir || true)"
    [ -n "$SYSTEMD_DIR" ] || return 1
    WANTS_DIR="$SYSTEMD_DIR/multi-user.target.wants"

    mkdir -p "$WANTS_DIR" 2>/dev/null || return 1
    remove_stale_etc_unit
    cp -f "$SIMPLEADMIN_DIR/systemd/$SYSTEMD_UNIT" "$SYSTEMD_DIR/$SYSTEMD_UNIT" || return 1
    link_unit "$SYSTEMD_UNIT" || return 1
    SERVICE_UNIT_INSTALLED="1"
    log "systemd 服务已安装: $SYSTEMD_DIR/$SYSTEMD_UNIT"
    return 0
}

start_fallback_service() {
    log "正在使用 /usrdata 后台模式启动 SimpleAdmin Rust"
    "$FALLBACK_STOP_SCRIPT" >/dev/null 2>&1 || true
    if "$FALLBACK_START_SCRIPT" >/tmp/simpleadmin-fallback-start.out 2>&1; then
        log "后台进程启动完成"
    else
        if grep -Eq "80 端口已被占用|80 端口仍被占用|port 80 is already in use" /tmp/simpleadmin-fallback-start.out 2>/dev/null; then
            warn "80 端口已被占用，SimpleAdmin Rust 无法绑定 :80"
            cat /tmp/simpleadmin-fallback-start.out 2>/dev/null || true
            fail "请先停止占用 80 端口的旧 Web 服务后重新安装"
        fi
        warn "后台启动失败，详情请查看 /tmp/simpleadmin-fallback-start.out"
        fail "simpleadmin-httpd 后台启动失败"
    fi
}

restart_services() {
    if [ "$SERVICE_UNIT_INSTALLED" = "1" ] && command -v systemctl >/dev/null 2>&1; then
        log "正在重载 systemd 并启动 SimpleAdmin Rust 服务（AT 调试关闭）"
        systemctl daemon-reload >/dev/null 2>&1 || true
        systemctl enable "$SYSTEMD_UNIT" >/dev/null 2>&1 || true
        systemctl restart "$SYSTEMD_UNIT" >/dev/null 2>&1 || warn "服务启动失败或当前设备不支持: $SYSTEMD_UNIT"
        if wait_for_web && systemctl is-active "$SYSTEMD_UNIT" >/dev/null 2>&1; then
            echo "[成功] $SYSTEMD_UNIT 已启动"
            remove_post_boot_autostart
            return 0
        fi
        warn "$SYSTEMD_UNIT 未处于 active 状态，改用 post_boot 自启动"
    fi

    if [ "$SERVICE_UNIT_INSTALLED" != "1" ]; then
        warn "systemd 服务未安装到 /lib/systemd/system，改用 post_boot 自启动"
    fi
    systemctl stop "$SYSTEMD_UNIT" >/dev/null 2>&1 || true
    systemctl disable "$SYSTEMD_UNIT" >/dev/null 2>&1 || true
    remove_systemd_unit_files
    systemctl daemon-reload >/dev/null 2>&1 || true
    install_post_boot_autostart || fail "无法安装开机自启动，请检查固件的 systemd/post_boot 支持"
    start_fallback_service
    log "正在检查服务启动状态:"
    if wait_for_web; then
        echo "[成功] simpleadmin-httpd 进程已运行"
    else
        fail "simpleadmin-httpd 启动失败"
    fi
}

reset_install_runtime_markers() {
    rm -f "$REBOOT_MARKER_FILE" "$INSTALL_RESULT_FILE" 2>/dev/null || true
}

write_reboot_marker_if_mobileap_cfg_touched() {
    rm -f "$REBOOT_MARKER_FILE" "$INSTALL_RESULT_FILE" 2>/dev/null || true
    # Persisting the live bridge MAC needs no runtime network changes.
    echo "REBOOT_REQUIRED=0" > "$INSTALL_RESULT_FILE" 2>/dev/null || true
}

main() {
    [ -d "$SIMPLEADMIN_SRC" ] || fail "安装包不完整: $SIMPLEADMIN_SRC"
    log "开始安装 SimpleAdmin Rust 和 Rust 原生 SMD AT 服务"
    reset_install_runtime_markers
    preflight
    remount_rw
    install_simpleadmin_files
    install_at_device_config
    install_ttl_state
    SIMPLEADMIN_MANAGE_ROOTFS=0 "$SIMPLEADMIN_DIR/simpleadmin-httpd" root-password-init || fail "初始化系统 root 密码失败"
    maybe_install_bridge0_mac_config
    restart_services
    remount_ro
    write_reboot_marker_if_mobileap_cfg_touched
    write_install_success_result
    log "安装完成。首次安装默认 admin / admin；升级保留原 Web 登录密码"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
    main "$@"
fi
