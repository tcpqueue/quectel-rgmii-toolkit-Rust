#!/bin/sh

# Read data only; never execute a configuration file as shell code.
simpleadmin_valid_port() {
    case "$1" in ''|0*|*[!0-9]*) return 1 ;; esac
    [ "${#1}" -le 5 ] && [ "$1" -le 65535 ]
}

simpleadmin_read_http_port() (
    file="${1:-/usrdata/simpleadmin/http_port}"
    port=80
    if [ -e "$file" ]; then
        port="$(cat "$file")" || return 1
    fi
    if ! simpleadmin_valid_port "$port"; then
        echo "[错误] HTTP 端口配置无效，请在安装器中指定 1–65535 的端口重新安装" >&2
        return 1
    fi
    printf '%s\n' "$port"
)
