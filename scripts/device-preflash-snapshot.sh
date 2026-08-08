#!/system/bin/sh
# REHYBIRD / Hybrid Mount pre-flash baseline snapshot.
# Read-only: no mount, module, service, property, radio, Bluetooth, or LKM state is changed.

set -u

OUT_BASE="${1:-/sdcard/Download}"
STAMP="$(date +%Y%m%d_%H%M%S 2>/dev/null || echo unknown)"
OUT_DIR="${OUT_BASE}/REHYBIRD_PreFlash_${STAMP}"
ARCHIVE="${OUT_DIR}.tar.gz"
MODULE_DIR="/data/adb/modules/hybrid_mount"

umask 077
mkdir -p "$OUT_DIR" || exit 1

have() {
  command -v "$1" >/dev/null 2>&1
}

redact_stream() {
  if have sed; then
    sed -E \
      -e 's/([0-9A-Fa-f]{2}:){5}[0-9A-Fa-f]{2}/<REDACTED_MAC>/g' \
      -e 's/\b[0-9]{14,20}\b/<REDACTED_LONG_ID>/g' \
      -e 's/(ro\.serialno|ro\.boot\.serialno|ril\.serialnumber|persist\.radio\.imei|gsm\.sim\.operator\.numeric|gsm\.operator\.numeric)([^]]*]|[=: ]+)[^ ]+/# <REDACTED_PROPERTY>/Ig'
  else
    cat
  fi
}

capture() {
  file="$1"
  shift
  {
    echo "# command: $*"
    echo "# captured: $(date -Iseconds 2>/dev/null || date)"
    "$@"
    echo "# exit_status=$?"
  } 2>&1 | redact_stream >"$OUT_DIR/$file"
}

capture device.txt sh -c '
  echo "boot_completed=$(getprop sys.boot_completed)"
  echo "android_release=$(getprop ro.build.version.release)"
  echo "sdk=$(getprop ro.build.version.sdk)"
  echo "fingerprint=$(getprop ro.build.fingerprint)"
  echo "product=$(getprop ro.product.device)"
  echo "kernel=$(uname -a)"
  echo "selinux=$(getenforce 2>/dev/null || echo unknown)"
'

capture module-state.txt sh -c '
  MOD="/data/adb/modules/hybrid_mount"
  if [ -d "$MOD" ]; then
    ls -ld "$MOD"
    for marker in disable remove update skip_mount; do
      [ -e "$MOD/$marker" ] && echo "marker:$marker=present" || echo "marker:$marker=absent"
    done
    if [ -f "$MOD/module.prop" ]; then
      grep -E "^(id|name|version|versionCode|author|description|metamodule|updateJson|webuiIcon)=" "$MOD/module.prop" || true
    fi
  else
    echo "module_dir_missing=1"
  fi
'

capture kasumi-lkm.txt sh -c '
  uname -r
  grep -Ei "(^|[[:space:]])(kasumi|kasumi_lkm)([[:space:]]|$)" /proc/modules 2>/dev/null || true
  ls -ld /sys/module/kasumi /sys/module/kasumi_lkm 2>/dev/null || true
'

capture mounts.txt sh -c '
  grep -Ei "hybrid[-_]?mount|rehybird|kasumi|overlay" /proc/self/mountinfo 2>/dev/null || true
'

capture bluetooth.txt sh -c '
  service list 2>/dev/null | grep -Ei "bluetooth" || true
  getprop 2>/dev/null | grep -Ei "\[init\.svc\..*bluetooth|\[init\.svc\..*bt" || true
  if command -v dumpsys >/dev/null 2>&1; then
    dumpsys bluetooth_manager 2>/dev/null \
      | grep -Ei "enabled:|state:|mState|Bluetooth Status|crash|error|fatal" \
      | head -n 120 || true
  fi
'

capture radio.txt sh -c '
  service list 2>/dev/null | grep -Ei "phone|radio|telephony|ims" || true
  getprop 2>/dev/null \
    | grep -Ei "\[init\.svc\..*(ril|radio|ims|qcril|rild|vendor.*radio)" \
    | head -n 220 || true
  for key in gsm.version.baseband gsm.network.type gsm.operator.alpha gsm.sim.state persist.radio.multisim.config; do
    value="$(getprop "$key" 2>/dev/null)"
    [ -n "$value" ] && echo "$key=$value"
  done
'

{
  echo "REHYBIRD PRE-FLASH BASELINE"
  echo "captured_at=$(date -Iseconds 2>/dev/null || date)"
  echo "boot_completed=$(getprop sys.boot_completed 2>/dev/null)"
  echo "kernel_release=$(uname -r 2>/dev/null)"
  echo "module_present=$([ -d "$MODULE_DIR" ] && echo yes || echo no)"
  echo "module_disabled=$([ -e "$MODULE_DIR/disable" ] && echo yes || echo no)"
  echo "kasumi_loaded=$(grep -Eq "(^|[[:space:]])(kasumi|kasumi_lkm)([[:space:]]|$)" /proc/modules 2>/dev/null && echo yes || echo no)"
  echo "bluetooth_service=$(service list 2>/dev/null | grep -Eqi bluetooth && echo present || echo missing)"
  echo "telephony_service=$(service list 2>/dev/null | grep -Eqi "phone|radio|telephony" && echo present || echo missing)"
  echo "baseband_property=$([ -n "$(getprop gsm.version.baseband 2>/dev/null)" ] && echo present || echo missing)"
} >"$OUT_DIR/BASELINE.env"

if have sha256sum; then
  (
    cd "$OUT_DIR" || exit 1
    find . -maxdepth 1 -type f ! -name SHA256SUMS.txt -print0 \
      | sort -z \
      | xargs -0 sha256sum > SHA256SUMS.txt
  )
fi

if have tar; then
  tar -czf "$ARCHIVE" -C "$OUT_BASE" "$(basename "$OUT_DIR")" 2>/dev/null || true
fi

cat "$OUT_DIR/BASELINE.env"
echo
echo "Baseline directory: $OUT_DIR"
if [ -f "$ARCHIVE" ]; then
  echo "Archive: $ARCHIVE"
  have sha256sum && sha256sum "$ARCHIVE"
fi
