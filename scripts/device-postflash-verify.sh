#!/system/bin/sh
# REHYBIRD / Hybrid Mount controlled post-flash verifier.
# Read-only: this script does not mount, unmount, load/unload LKMs, edit props,
# restart services, or modify module state.

set -u

OUT_BASE="${1:-/sdcard/Download}"
STAMP="$(date +%Y%m%d_%H%M%S 2>/dev/null || echo unknown)"
WORK="${TMPDIR:-/data/local/tmp}/rehybird-postflash-${STAMP}-$$"
OUT_DIR="${OUT_BASE}/REHYBIRD_PostFlash_${STAMP}"
ARCHIVE="${OUT_DIR}.tar.gz"
MODULE_DIR="/data/adb/modules/hybrid_mount"
PRIVATE_DIR="/data/adb/hybrid-mount"

umask 077
mkdir -p "$WORK" "$OUT_DIR" || exit 1

cleanup() {
  rm -rf "$WORK" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

have() {
  command -v "$1" >/dev/null 2>&1
}

safe_run() {
  file="$1"
  shift
  {
    echo "# command: $*"
    echo "# captured: $(date -Iseconds 2>/dev/null || date)"
    "$@"
    rc=$?
    echo
    echo "# exit_status=$rc"
  } >"$WORK/$file" 2>&1
}

# Conservative text redaction. It intentionally redacts common Android IDs,
# MAC addresses, IPv6 addresses, and long decimal identifiers before export.
redact_file() {
  src="$1"
  dst="$2"
  if have sed; then
    sed -E \
      -e 's/([0-9A-Fa-f]{2}:){5}[0-9A-Fa-f]{2}/<REDACTED_MAC>/g' \
      -e 's/\b[0-9]{14,20}\b/<REDACTED_LONG_ID>/g' \
      -e 's/([A-Fa-f0-9]{4}:){2,7}[A-Fa-f0-9]{1,4}/<REDACTED_IPV6>/g' \
      -e 's/(ro\.serialno|ro\.boot\.serialno|ril\.serialnumber|persist\.radio\.imei|gsm\.sim\.operator\.numeric|gsm\.operator\.numeric)([^]]*]|[=: ]+)[^ ]+/# <REDACTED_PROPERTY>/Ig' \
      "$src" >"$dst" 2>/dev/null || cp "$src" "$dst"
  else
    cp "$src" "$dst"
  fi
}

record_text() {
  name="$1"
  src="$WORK/$name"
  [ -f "$src" ] || return 0
  redact_file "$src" "$OUT_DIR/$name"
}

# 1. Immutable device/build identity needed for compatibility decisions.
safe_run device.txt sh -c '
  echo "boot_completed=$(getprop sys.boot_completed)"
  echo "android_release=$(getprop ro.build.version.release)"
  echo "sdk=$(getprop ro.build.version.sdk)"
  echo "fingerprint=$(getprop ro.build.fingerprint)"
  echo "product=$(getprop ro.product.device)"
  echo "manufacturer=$(getprop ro.product.manufacturer)"
  echo "kernel=$(uname -a)"
  echo "selinux=$(getenforce 2>/dev/null || echo unknown)"
'
record_text device.txt

# 2. Root/module state. Only names, markers, metadata, modes and timestamps.
safe_run module-state.txt sh -c '
  MOD="/data/adb/modules/hybrid_mount"
  PRIV="/data/adb/hybrid-mount"
  echo "module_dir=$MOD"
  if [ -d "$MOD" ]; then
    ls -ld "$MOD"
    for marker in disable remove update skip_mount; do
      if [ -e "$MOD/$marker" ]; then
        echo "marker:$marker=present"
      else
        echo "marker:$marker=absent"
      fi
    done
    if [ -f "$MOD/module.prop" ]; then
      echo "--- module.prop ---"
      grep -E "^(id|name|version|versionCode|author|description|metamodule|updateJson|webuiIcon)=" "$MOD/module.prop" || true
    fi
  else
    echo "module_dir_missing=1"
  fi
  echo "--- private-state metadata ---"
  if [ -d "$PRIV" ]; then
    find "$PRIV" -maxdepth 2 -mindepth 1 -printf "%M %u:%g %s %TY-%Tm-%TdT%TH:%TM:%TS %p\n" 2>/dev/null | sort
  else
    echo "private_state_missing=1"
  fi
'
record_text module-state.txt

# 3. Kernel module status. Do not load or unload anything.
safe_run kasumi-lkm.txt sh -c '
  echo "--- uname ---"
  uname -r
  echo "--- /proc/modules matches ---"
  grep -Ei "(^|[[:space:]])(kasumi|kasumi_lkm)([[:space:]]|$)" /proc/modules 2>/dev/null || true
  echo "--- module sysfs matches ---"
  ls -ld /sys/module/kasumi /sys/module/kasumi_lkm 2>/dev/null || true
  echo "--- packaged LKM set ---"
  find /data/adb/modules/hybrid_mount/kasumi_lkm -maxdepth 1 -type f -name "*.ko" -printf "%f %s bytes\n" 2>/dev/null | sort || true
'
record_text kasumi-lkm.txt

# 4. Mount state. Restricted to Hybrid Mount/Kasumi/overlay-related lines.
safe_run mounts.txt sh -c '
  echo "--- mountinfo relevant lines ---"
  grep -Ei "hybrid[-_]?mount|rehybird|kasumi|overlay" /proc/self/mountinfo 2>/dev/null || true
  echo "--- mounts relevant lines ---"
  grep -Ei "hybrid[-_]?mount|rehybird|kasumi|overlay" /proc/mounts 2>/dev/null || true
  echo "--- loop devices owned by private state ---"
  for loop in /sys/block/loop*/loop/backing_file; do
    [ -r "$loop" ] || continue
    value="$(cat "$loop" 2>/dev/null)"
    case "$value" in
      *hybrid-mount*|*rehybird*) echo "$loop -> $value" ;;
    esac
  done
'
record_text mounts.txt

# 5. Bluetooth health without paired-device dumps.
safe_run bluetooth.txt sh -c '
  echo "--- service presence ---"
  service list 2>/dev/null | grep -Ei "bluetooth" || true
  echo "--- init services ---"
  getprop 2>/dev/null | grep -Ei "\[init\.svc\..*bluetooth|\[init\.svc\..*bt" || true
  echo "--- manager summary ---"
  if command -v dumpsys >/dev/null 2>&1; then
    dumpsys bluetooth_manager 2>/dev/null \
      | grep -Ei "enabled:|state:|mState|Bluetooth Status|name:|address:|crash|error|fatal" \
      | head -n 160 || true
  fi
'
record_text bluetooth.txt

# 6. Radio/baseband health. Avoid full telephony dumps because they may expose
# subscriber identifiers. Capture service state and generic network state only.
safe_run radio.txt sh -c '
  echo "--- telephony/radio service presence ---"
  service list 2>/dev/null | grep -Ei "phone|radio|telephony|ims" || true
  echo "--- init radio services ---"
  getprop 2>/dev/null \
    | grep -Ei "\[init\.svc\..*(ril|radio|ims|qcril|rild|vendor.*radio)" \
    | head -n 240 || true
  echo "--- generic radio properties ---"
  for key in \
    gsm.version.baseband \
    gsm.network.type \
    gsm.operator.alpha \
    gsm.sim.state \
    persist.radio.multisim.config; do
    value="$(getprop "$key" 2>/dev/null)"
    [ -n "$value" ] && echo "$key=$value"
  done
'
record_text radio.txt

# 7. Relevant error evidence only. A bounded tail avoids exporting the full
# logcat/dmesg history and reduces privacy exposure.
if have logcat; then
  safe_run logcat-relevant.raw sh -c '
    logcat -d -v threadtime -t 2500 2>/dev/null \
      | grep -Ei "hybrid[-_]?mount|rehybird|kasumi|bluetooth|bt_stack|qcril|rild|radio|modem|fatal|panic" \
      | tail -n 600 || true
  '
  record_text logcat-relevant.raw
fi

if have dmesg; then
  safe_run dmesg-relevant.raw sh -c '
    dmesg 2>/dev/null \
      | grep -Ei "hybrid[-_]?mount|rehybird|kasumi|bluetooth|qcril|rild|radio|modem|oops|panic|BUG:" \
      | tail -n 500 || true
  '
  record_text dmesg-relevant.raw
fi

# 8. Deterministic verdict. Keep it conservative: PASS means only that the
# observable gates below passed, not that every hardware path is proven safe.
{
  echo "REHYBIRD POST-FLASH VERIFICATION"
  echo "captured_at=$(date -Iseconds 2>/dev/null || date)"
  echo

  failed=0
  boot="$(getprop sys.boot_completed 2>/dev/null)"
  if [ "$boot" = "1" ]; then
    echo "PASS boot_completed"
  else
    echo "FAIL boot_completed value=${boot:-empty}"
    failed=1
  fi

  if [ -d "$MODULE_DIR" ]; then
    echo "PASS module_present"
  else
    echo "FAIL module_present"
    failed=1
  fi

  if [ -e "$MODULE_DIR/disable" ]; then
    echo "FAIL module_disabled"
    failed=1
  else
    echo "PASS module_not_disabled"
  fi

  if grep -Eq "(^|[[:space:]])(kasumi|kasumi_lkm)([[:space:]]|$)" /proc/modules 2>/dev/null; then
    echo "INFO kasumi_lkm=loaded"
  else
    echo "INFO kasumi_lkm=not_loaded_or_integrated"
  fi

  if grep -Eqi "hybrid[-_]?mount|rehybird|kasumi|overlay" /proc/self/mountinfo 2>/dev/null; then
    echo "PASS relevant_mounts_observed"
  else
    echo "WARN relevant_mounts_not_observed"
  fi

  if service list 2>/dev/null | grep -Eqi "bluetooth"; then
    echo "PASS bluetooth_service_present"
  else
    echo "FAIL bluetooth_service_present"
    failed=1
  fi

  if service list 2>/dev/null | grep -Eqi "phone|radio|telephony"; then
    echo "PASS telephony_service_present"
  else
    echo "FAIL telephony_service_present"
    failed=1
  fi

  baseband="$(getprop gsm.version.baseband 2>/dev/null)"
  if [ -n "$baseband" ] && [ "$baseband" != "unknown" ]; then
    echo "PASS baseband_property_present"
  else
    echo "WARN baseband_property_missing"
  fi

  echo
  if [ "$failed" -eq 0 ]; then
    echo "VERDICT=OBSERVABLE_GATES_PASS"
  else
    echo "VERDICT=OBSERVABLE_GATES_FAIL"
  fi
  echo "NOTE=Manual Bluetooth audio/call/mobile-data checks are still required."
} >"$OUT_DIR/VERDICT.txt"

# Hash every exported text file, then package it. No secrets or module payloads
# are copied into the report.
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

cat "$OUT_DIR/VERDICT.txt"
echo
echo "Report directory: $OUT_DIR"
if [ -f "$ARCHIVE" ]; then
  echo "Archive: $ARCHIVE"
  if have sha256sum; then
    sha256sum "$ARCHIVE"
  fi
fi
