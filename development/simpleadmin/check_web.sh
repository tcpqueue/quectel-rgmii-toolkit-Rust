#!/bin/bash
set -u
. "$(dirname "$0")/web_port.sh"
port="${SIMPLEADMIN_CHECK_PORT:-$(simpleadmin_read_http_port)}"
simpleadmin_valid_port "$port" || exit 1

# Loopback only: no login, AT commands, or modem configuration changes.
check_page() (
    local path="$1" marker="$2" status="" line="" body="" deadline redirect=0 location=0
    exec 3<>/dev/tcp/127.0.0.1/"$port" || exit 1
    printf 'GET %s HTTP/1.0\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n' "$path" >&3
    IFS= read -r -t 2 status <&3 || exit 1
    case "$status" in
        "HTTP/1.0 200 "*|"HTTP/1.1 200 "*) ;;
        "HTTP/1.0 303 "*|"HTTP/1.1 303 "*) [ "$path" = / ] || exit 1; redirect=1 ;;
        *) exit 1 ;;
    esac
    deadline=$((SECONDS + 3))
    while IFS= read -r -t 1 line <&3; do
        [ "$SECONDS" -lt "$deadline" ] || exit 1
        case "$line" in [Ll][Oo][Cc][Aa][Tt][Ii][Oo][Nn]:\ /login.html*) location=1 ;; esac
        [ "$line" = $'\r' ] && break
    done
    if [ "$redirect" = 1 ]; then
        [ "$location" = 1 ]
        exit "$?"
    fi
    deadline=$((SECONDS + 5))
    while IFS= read -r -t 2 line <&3 || [ -n "$line" ]; do
        body="$body$line"
        [[ "$body" == *"$marker"* ]] && exit 0
        [ "${#body}" -le 262144 ] && [ "$SECONDS" -lt "$deadline" ] || exit 1
    done
    exit 1
)

for page in '/:SimpleAdminSpaMode' '/login.html:loginLanguage' '/js/locales.js:root.Lang'; do
    if ! check_page "${page%%:*}" "${page#*:}" 2>/dev/null; then
        echo "WEB_CHECK=FAIL path=${page%%:*} (connection, HTTP status, or content)" >&2
        exit 1
    fi
done
echo 'WEB_CHECK=OK'
