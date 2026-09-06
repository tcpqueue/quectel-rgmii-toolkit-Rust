#!/bin/sh
# Match the actual executable, never a PID file or a substring of `ps` output.
runtime_kind() {
    case "$1" in ''|*[!0-9]*|0|1) return 1 ;; esac
    runtime_exe="$(readlink "/proc/$1/exe" 2>/dev/null || true)"
    [ -n "$runtime_exe" ] || return 1
    case "$runtime_exe" in
        /usrdata/simpleadmin/simpleadmin-httpd|'/usrdata/simpleadmin/simpleadmin-httpd (deleted)') echo server ;;
        */cat|*/socat|*/socat-armel-static)
            runtime_args="$(tr '\000' '\n' < "/proc/$1/cmdline" 2>/dev/null)" || return 1
            case "$runtime_exe" in
                */cat) printf '%s\n' "$runtime_args" | grep -qx '/dev/ttyIN' || return 1 ;;
                *) printf '%s\n' "$runtime_args" | grep -Eq '^([^ ]*,)?link=/dev/tty(IN|OUT)(,|$)' || return 1 ;;
            esac
            echo bridge ;;
        *) return 1 ;;
    esac
}

runtime_pids() {
    for runtime_entry in /proc/[0-9]*; do
        runtime_pid="${runtime_entry##*/}"
        [ "$(runtime_kind "$runtime_pid" || true)" = "$1" ] && echo "$runtime_pid"
    done
    return 0
}

stop_runtime_kind() {
    for runtime_target in $(runtime_pids "$1"); do
        runtime_birth="$(awk '{print $22}' "/proc/$runtime_target/stat" 2>/dev/null || true)"
        [ -n "$runtime_birth" ] || continue
        [ "$(runtime_kind "$runtime_target" || true)" = "$1" ] || continue
        echo "[信息] 停止已核对的 $1 进程: pid=$runtime_target"
        kill "$runtime_target" 2>/dev/null || true
        sleep 1
        [ "$(runtime_kind "$runtime_target" || true)" = "$1" ] || continue
        [ "$(awk '{print $22}' "/proc/$runtime_target/stat" 2>/dev/null || true)" = "$runtime_birth" ] || continue
        kill -9 "$runtime_target" 2>/dev/null || true
    done
}

cleanup_legacy_at_bridges() {
    stop_runtime_kind bridge
    for runtime_link in /dev/ttyIN /dev/ttyOUT; do
        [ ! -L "$runtime_link" ] || rm -f "$runtime_link"
    done
}
