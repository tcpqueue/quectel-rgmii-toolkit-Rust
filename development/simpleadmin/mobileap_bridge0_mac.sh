#!/bin/bash
set -eu

SIMPLEADMIN_DIR="${SIMPLEADMIN_DIR:-/usrdata/simpleadmin}"
MOBILEAP_RESULT_FILE="${MOBILEAP_RESULT_FILE:-/tmp/simpleadmin-mobileap-result.env}"
MOBILEAP_CFG_PRIMARY="/usrdata/etc/data/mobileap_cfg.xml"
MOBILEAP_CFG_FALLBACK="/etc/data/mobileap_cfg.xml"
BRIDGE0_ADDRESS_FILE="/sys/class/net/bridge0/address"
restore_ro=0
tmp=""
cleanup() {
    [ -z "$tmp" ] || rm -f "$tmp"
    if [ "$restore_ro" = 1 ]; then
        sync
        mount -o remount,ro / || return 1
    fi
}
trap cleanup EXIT
echo 'MOBILEAP_CFG_TOUCHED=0' > "$MOBILEAP_RESULT_FILE"
case "${SIMPLEADMIN_FIX_BRIDGE0_MAC:-1}" in
    0|no|NO|false|FALSE|off|OFF) echo '[信息] 已跳过固定 bridge0 MAC'; exit 0 ;;
esac

# Capture the live address; never generate or apply a different MAC.
mac="$(tr 'a-f' 'A-F' 2>/dev/null < "$BRIDGE0_ADDRESS_FILE")" || mac=""
if ! [[ "$mac" =~ ^[0-9A-F]{2}(:[0-9A-F]{2}){5}$ ]] ||
   [ "$mac" = "00:00:00:00:00:00" ] ||
   [ "$((16#${mac:0:2} & 1))" != 0 ]; then
    echo '[警告] 无法读取有效的 bridge0 MAC，跳过固定，不生成随机地址'
    exit 0
fi

cfg="$MOBILEAP_CFG_PRIMARY"
[ -f "$cfg" ] || cfg="$MOBILEAP_CFG_FALLBACK"
if [ ! -s "$cfg" ]; then
    echo '[警告] 未找到有效 QCMAP 配置，无法保证重启后 MAC 固定'
    exit 0
fi
# Only replace existing fields; unknown firmware formats remain untouched.
if grep -q '<APMACAddress>[^<]*</APMACAddress>' "$cfg"; then
    kind=apmac
elif grep -q '<EarlyEthMode>[^<]*</EarlyEthMode>' "$cfg" &&
     grep -q '<EarlyEthMACAddr>[^<]*</EarlyEthMACAddr>' "$cfg"; then
    kind=earlyeth
else
    echo '[警告] 固件没有支持的 MAC 配置字段，跳过固定'
    exit 0
fi

tmp="$(mktemp /tmp/simpleadmin-mac.XXXXXX)"
if [ "$kind" = apmac ]; then
    sed "s#<APMACAddress>[^<]*</APMACAddress>#<APMACAddress>${mac}</APMACAddress>#g" "$cfg" > "$tmp"
else
    sed -e 's#<EarlyEthMode>[^<]*</EarlyEthMode>#<EarlyEthMode>1</EarlyEthMode>#g' \
        -e "s#<EarlyEthMACAddr>[^<]*</EarlyEthMACAddr>#<EarlyEthMACAddr>${mac}</EarlyEthMACAddr>#g" "$cfg" > "$tmp"
fi
if cmp -s "$cfg" "$tmp"; then
    echo "[信息] 当前 bridge0 MAC 已固定：$mac，无需写入或重启"
    exit 0
fi

# Standalone use also observes rw -> write/sync -> ro. During installation
# the parent already owns this transition.
if awk '$2 == "/" && $4 ~ /(^|,)ro(,|$)/ { found=1 } END { exit !found }' /proc/mounts; then
    restore_ro=1
    mount -o remount,rw /
fi
backup="$cfg.simpleadmin.bak"
[ -e "$backup" ] || cp -p "$cfg" "$backup"
staged="$cfg.simpleadmin.tmp.$$"
# Preserve firmware ownership and mode rather than imposing radio:radio/0755.
cp -p "$cfg" "$staged"
if ! cat "$tmp" > "$staged" || ! cmp -s "$tmp" "$staged"; then
    rm -f "$staged"
    echo '[警告] MAC 配置写入校验失败，原配置未替换'
    exit 1
fi
mv -f "$staged" "$cfg"
sync
{
    echo 'MOBILEAP_CFG_TOUCHED=1'
    echo "BRIDGE0_MAC=$mac"
} > "$MOBILEAP_RESULT_FILE"
echo "[信息] 已保存当前 bridge0 MAC：$mac；不切换当前地址、不重启，下次启动沿用"
