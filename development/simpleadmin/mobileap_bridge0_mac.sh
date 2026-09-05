#!/bin/bash

set -u

SIMPLEADMIN_DIR="${SIMPLEADMIN_DIR:-/usrdata/simpleadmin}"
MOBILEAP_RESULT_FILE="${MOBILEAP_RESULT_FILE:-/tmp/simpleadmin-mobileap-result.env}"
MOBILEAP_CFG_PRIMARY="/usrdata/etc/data/mobileap_cfg.xml"
MOBILEAP_CFG_FALLBACK="/etc/data/mobileap_cfg.xml"
MOBILEAP_CFG_FILE="$MOBILEAP_CFG_FALLBACK"
MOBILEAP_CFG_BACKUP="$MOBILEAP_CFG_FILE.simpleadmin.bak"
BRIDGE0_MAC_FILE="$SIMPLEADMIN_DIR/bridge0_mac"
BRIDGE0_MAC_PREFIX="06:3F:B1"
MOBILEAP_CFG_OWNER="radio:radio"
MOBILEAP_CFG_MODE="0755"
MOBILEAP_CFG_TOUCHED="0"
BRIDGE0_MAC_VALUE=""

log() {
    echo "[信息] $*"
}

warn() {
    echo "[警告] $*"
}

write_result() {
    {
        echo "MOBILEAP_CFG_TOUCHED=${MOBILEAP_CFG_TOUCHED:-0}"
        echo "BRIDGE0_MAC=${BRIDGE0_MAC_VALUE:-}"
    } > "$MOBILEAP_RESULT_FILE" 2>/dev/null || true
}

is_simpleadmin_bridge0_mac() {
    local mac="$1"
    echo "$mac" | grep -Eq "^${BRIDGE0_MAC_PREFIX}:[0-9A-F]{2}:[0-9A-F]{2}:[0-9A-F]{2}$"
}

normalize_mac() {
    echo "$1" | tr -d ' \r\n\t' | tr 'a-f' 'A-F'
}

select_mobileap_cfg_file() {
    if [ -f "$MOBILEAP_CFG_PRIMARY" ] || [ -f "$MOBILEAP_CFG_PRIMARY.simpleadmin.bak" ]; then
        MOBILEAP_CFG_FILE="$MOBILEAP_CFG_PRIMARY"
    elif [ -f "$MOBILEAP_CFG_FALLBACK" ]; then
        MOBILEAP_CFG_FILE="$MOBILEAP_CFG_FALLBACK"
    else
        MOBILEAP_CFG_FILE="$MOBILEAP_CFG_PRIMARY"
    fi
    MOBILEAP_CFG_BACKUP="$MOBILEAP_CFG_FILE.simpleadmin.bak"
}

restore_mobileap_cfg_permissions() {
    [ -f "$MOBILEAP_CFG_FILE" ] || return 0
    chown "$MOBILEAP_CFG_OWNER" "$MOBILEAP_CFG_FILE" 2>/dev/null || warn "恢复 QCMAP 配置属主失败: $MOBILEAP_CFG_OWNER"
    chmod "$MOBILEAP_CFG_MODE" "$MOBILEAP_CFG_FILE" 2>/dev/null || warn "恢复 QCMAP 配置权限失败: $MOBILEAP_CFG_MODE"
}

restore_mobileap_cfg_from_backup_if_needed() {
    select_mobileap_cfg_file
    if [ ! -s "$MOBILEAP_CFG_FILE" ] && [ -s "$MOBILEAP_CFG_BACKUP" ]; then
        warn "QCMAP 配置缺失或为空，正在从备份恢复: $MOBILEAP_CFG_BACKUP"
        cp -p -f "$MOBILEAP_CFG_BACKUP" "$MOBILEAP_CFG_FILE" || {
            warn "从备份恢复 QCMAP 配置失败"
            return 1
        }
        MOBILEAP_CFG_TOUCHED="1"
        restore_mobileap_cfg_permissions
    fi
}

read_saved_bridge0_mac() {
    local mac=""

    select_mobileap_cfg_file
    restore_mobileap_cfg_from_backup_if_needed >/dev/null 2>&1 || true

    if [ -f "$BRIDGE0_MAC_FILE" ]; then
        mac="$(normalize_mac "$(cat "$BRIDGE0_MAC_FILE" 2>/dev/null || true)")"
        if is_simpleadmin_bridge0_mac "$mac"; then
            echo "$mac"
            return 0
        fi
    fi

    if [ -f "$MOBILEAP_CFG_FILE" ]; then
        mac="$(sed -n 's#.*<APMACAddress>\([^<]*\)</APMACAddress>.*#\1#p' "$MOBILEAP_CFG_FILE" 2>/dev/null | head -n 1)"
        mac="$(normalize_mac "$mac")"
        if is_simpleadmin_bridge0_mac "$mac"; then
            echo "$mac"
            return 0
        fi

        mac="$(sed -n 's#.*<EarlyEthMACAddr>\([^<]*\)</EarlyEthMACAddr>.*#\1#p' "$MOBILEAP_CFG_FILE" 2>/dev/null | head -n 1)"
        mac="$(normalize_mac "$mac")"
        if is_simpleadmin_bridge0_mac "$mac"; then
            echo "$mac"
            return 0
        fi
    fi

    return 1
}

generate_bridge0_mac() {
    local hex=""
    local seed="0"

    hex="$(dd if=/dev/urandom bs=3 count=1 2>/dev/null | od -An -tx1 2>/dev/null | tr -d ' \n' | head -c 6)"
    hex="$(echo "$hex" | tr 'a-f' 'A-F')"

    if ! echo "$hex" | grep -Eq '^[0-9A-F]{6}$'; then
        seed="$(date +%s 2>/dev/null || echo 0)"
        hex="$(printf '%06X' $(( (seed ^ $$ ^ RANDOM) & 0xFFFFFF )))"
    fi

    printf '%s:%s:%s:%s\n' "$BRIDGE0_MAC_PREFIX" "${hex:0:2}" "${hex:2:2}" "${hex:4:2}"
}

write_mobileap_cfg_tmp() {
    local tmp="$1"
    local kind="$2"
    local mac="$3"

    case "$kind" in
        apmac)
            sed "s#<APMACAddress>[^<]*</APMACAddress>#<APMACAddress>${mac}</APMACAddress>#" "$MOBILEAP_CFG_FILE" > "$tmp"
            ;;
        earlyeth)
            sed \
                -e 's#<EarlyEthMode>[^<]*</EarlyEthMode>#<EarlyEthMode>1</EarlyEthMode>#' \
                -e "s#<EarlyEthMACAddr>[^<]*</EarlyEthMACAddr>#<EarlyEthMACAddr>${mac}</EarlyEthMACAddr>#" \
                "$MOBILEAP_CFG_FILE" > "$tmp"
            ;;
        *)
            return 1
            ;;
    esac
}

validate_mobileap_cfg_tmp() {
    local tmp="$1"
    local kind="$2"
    local mac="$3"

    [ -s "$tmp" ] || return 1
    case "$kind" in
        apmac)
            grep -q "<APMACAddress>${mac}</APMACAddress>" "$tmp" 2>/dev/null
            ;;
        earlyeth)
            grep -q '<EarlyEthMode>1</EarlyEthMode>' "$tmp" 2>/dev/null && grep -q "<EarlyEthMACAddr>${mac}</EarlyEthMACAddr>" "$tmp" 2>/dev/null
            ;;
        *)
            return 1
            ;;
    esac
}

set_mobileap_bridge0_mac() {
    local mac="$1"
    local tmp=""
    local kind=""

    select_mobileap_cfg_file
    restore_mobileap_cfg_from_backup_if_needed || true

    if [ ! -f "$MOBILEAP_CFG_FILE" ]; then
        warn "未找到 QCMAP 配置，跳过 bridge0 MAC 持久化: $MOBILEAP_CFG_FILE"
        return 0
    fi

    if [ ! -s "$MOBILEAP_CFG_FILE" ]; then
        warn "QCMAP 配置为空，跳过 bridge0 MAC 持久化: $MOBILEAP_CFG_FILE"
        return 0
    fi

    if grep -q '<APMACAddress>' "$MOBILEAP_CFG_FILE" 2>/dev/null; then
        kind="apmac"
        if grep -q "<APMACAddress>${mac}</APMACAddress>" "$MOBILEAP_CFG_FILE" 2>/dev/null; then
            log "QCMAP 配置 APMACAddress 已是目标 bridge0 MAC，跳过写入"
            return 0
        fi
    elif grep -q '<EarlyEthMode>' "$MOBILEAP_CFG_FILE" 2>/dev/null && grep -q '<EarlyEthMACAddr>' "$MOBILEAP_CFG_FILE" 2>/dev/null; then
        kind="earlyeth"
        if grep -q '<EarlyEthMode>1</EarlyEthMode>' "$MOBILEAP_CFG_FILE" 2>/dev/null && grep -q "<EarlyEthMACAddr>${mac}</EarlyEthMACAddr>" "$MOBILEAP_CFG_FILE" 2>/dev/null; then
            log "QCMAP 配置 EarlyEthMode/EarlyEthMACAddr 已是目标 bridge0 MAC，跳过写入"
            return 0
        fi
    else
        warn "QCMAP 配置中未找到支持的 bridge0 MAC 字段，跳过写入"
        warn "支持字段: APMACAddress，或 EarlyEthMode 搭配 EarlyEthMACAddr"
        restore_mobileap_cfg_permissions
        return 0
    fi

    if [ ! -f "$MOBILEAP_CFG_BACKUP" ]; then
        cp -p -f "$MOBILEAP_CFG_FILE" "$MOBILEAP_CFG_BACKUP" || warn "备份 QCMAP 配置失败: $MOBILEAP_CFG_BACKUP"
    fi

    tmp="${MOBILEAP_CFG_FILE}.simpleadmin.tmp.$$"
    if write_mobileap_cfg_tmp "$tmp" "$kind" "$mac" && validate_mobileap_cfg_tmp "$tmp" "$kind" "$mac"; then
        if mv -f "$tmp" "$MOBILEAP_CFG_FILE"; then
            MOBILEAP_CFG_TOUCHED="1"
            restore_mobileap_cfg_permissions
            case "$kind" in
                apmac) log "QCMAP 配置 APMACAddress 已更新" ;;
                earlyeth) log "QCMAP 配置 EarlyEthMode/EarlyEthMACAddr 已更新" ;;
            esac
        else
            warn "原子替换 QCMAP 配置失败"
            rm -f "$tmp" 2>/dev/null || true
            restore_mobileap_cfg_permissions
        fi
    else
        warn "生成有效 QCMAP 配置失败，原文件未替换"
        rm -f "$tmp" 2>/dev/null || true
        restore_mobileap_cfg_permissions
    fi
}

apply_bridge0_mac_now() {
    local mac="$1"
    local current=""

    if [ ! -e /sys/class/net/bridge0/address ]; then
        warn "当前未找到 bridge0；如果 QCMAP 配置已变更，目标 MAC 可能在重启后生效"
        return 0
    fi

    current="$(cat /sys/class/net/bridge0/address 2>/dev/null | tr 'a-f' 'A-F' || true)"
    if [ "$current" != "$mac" ]; then
        ip link set dev bridge0 address "$mac" >/dev/null 2>&1 || ifconfig bridge0 hw ether "$mac" >/dev/null 2>&1 || warn "立即应用 bridge0 MAC 失败；如果 QCMAP 配置已变更，目标 MAC 可能在重启后生效"
    fi

    current="$(cat /sys/class/net/bridge0/address 2>/dev/null | tr 'a-f' 'A-F' || true)"
    if [ "$current" = "$mac" ]; then
        log "当前 bridge0 MAC: $current"
    else
        warn "当前 bridge0 MAC 与目标不一致: $current"
    fi
}

install_bridge0_mac_config() {
    local mac=""
    local touched_before=""

    touched_before="$MOBILEAP_CFG_TOUCHED"
    mac="$(read_saved_bridge0_mac || true)"
    if [ -z "$mac" ]; then
        mac="$(generate_bridge0_mac)"
    fi
    BRIDGE0_MAC_VALUE="$mac"

    mkdir -p "$SIMPLEADMIN_DIR"
    echo "$mac" > "$BRIDGE0_MAC_FILE"
    chmod 0644 "$BRIDGE0_MAC_FILE"

    log "正在固定 bridge0 MAC: $mac"
    set_mobileap_bridge0_mac "$mac"

    if [ "$MOBILEAP_CFG_TOUCHED" != "$touched_before" ]; then
        warn "QCMAP 配置已变更；安装过程中跳过 QCMAP 重启，避免中断 AT"
    else
        log "QCMAP 配置未变化，mobileap 配置不需要重启"
    fi
    apply_bridge0_mac_now "$mac"
}

main() {
    write_result
    case "${SIMPLEADMIN_FIX_BRIDGE0_MAC:-1}" in
        0|no|NO|false|FALSE|off|OFF)
            log "SIMPLEADMIN_FIX_BRIDGE0_MAC 已禁用，跳过 QCMAP mobileap 配置修改"
            write_result
            exit 0
            ;;
    esac

    install_bridge0_mac_config
    write_result
}

main "$@"
