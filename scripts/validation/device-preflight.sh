#!/system/bin/sh
# REHYBIRD read-only device preflight collector.
#
# This script does NOT load/unload kernel modules, mount/unmount filesystems,
# change module state, write /data/adb, reboot, or install anything. Its only
# writes are a private report directory and tar.gz archive under /data/local/tmp.

set -u
umask 077

SCRIPT_VERSION="2026-08-07.1"
BASE_DIR="/data/local/tmp"
STAMP="$(date +%Y%m%d-%H%M%S 2>/dev/null || echo unknown-time)"
OUT_DIR="$BASE_DIR/rehybird-preflight-$STAMP"
ARCHIVE="$OUT_DIR.tar.gz"

say() {
  printf '%s\n' "$*"
}

fail() {
  say "ERROR: $*" >&2
  exit 1
}

capture() {
  name="$1"
  shift
  {
    printf '# command:'
    for arg in "$@"; do
      printf ' %s' "$arg"
    done
    printf '\n\n'
    "$@"
  } >"$OUT_DIR/$name" 2>&1 || true
}

capture_sh() {
  name="$1"
  command_text="$2"
  {
    printf '# command: %s\n\n' "$command_text"
    sh -c "$command_text"
  } >"$OUT_DIR/$name" 2>&1 || true
}

copy_readable() {
  source_path="$1"
  output_name="$2"
  if [ -r "$source_path" ]; then
    cat "$source_path" >"$OUT_DIR/$output_name" 2>/dev/null || true
  else
    printf 'not readable: %s\n' "$source_path" >"$OUT_DIR/$output_name"
  fi
}

hash_tree() {
  root="$1"
  output="$2"
  {
    printf '# root: %s\n' "$root"
    if [ ! -d "$root" ]; then
      printf 'directory not present\n'
      return
    fi
    find "$root" -type f 2>/dev/null | sort | while IFS= read -r file; do
      if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$file" 2>/dev/null || true
      else
        stat "$file" 2>/dev/null || true
      fi
    done
  } >"$OUT_DIR/$output" 2>&1
}

[ "$(id -u 2>/dev/null || echo -1)" = "0" ] || fail "run this script from a root shell"
mkdir -p "$OUT_DIR" || fail "cannot create $OUT_DIR"
chmod 700 "$OUT_DIR" 2>/dev/null || true

say "REHYBIRD read-only preflight"
say "Report directory: $OUT_DIR"

cat >"$OUT_DIR/README.txt" <<EOF
REHYBIRD read-only device preflight
script_version=$SCRIPT_VERSION
created_at=$STAMP

The collector did not install a module, load/unload an LKM, mount/unmount a
filesystem, change /data/adb, or reboot the device. It only wrote this report.
Device serial properties are intentionally not collected.
EOF

capture identity.txt id
capture uname.txt uname -a
capture uptime.txt cat /proc/uptime
capture date.txt date
capture df.txt df -h
capture mount_command.txt mount
capture proc_filesystems.txt cat /proc/filesystems
capture proc_modules.txt cat /proc/modules
capture selinux_mode.txt getenforce
capture boot_completed.txt getprop sys.boot_completed
capture slot_suffix.txt getprop ro.boot.slot_suffix

{
  for prop in \
    ro.product.manufacturer \
    ro.product.brand \
    ro.product.device \
    ro.product.model \
    ro.build.version.release \
    ro.build.version.sdk \
    ro.build.version.security_patch \
    ro.build.fingerprint \
    ro.vendor.build.fingerprint \
    ro.product.first_api_level \
    ro.vendor.api_level \
    ro.kernel.version \
    ro.boot.verifiedbootstate \
    ro.boot.vbmeta.device_state \
    ro.boot.flash.locked \
    ro.boot.selinux; do
    printf '%s=' "$prop"
    getprop "$prop" 2>/dev/null || true
  done
} >"$OUT_DIR/selected_properties.txt" 2>&1

copy_readable /proc/version proc_version.txt
copy_readable /proc/sys/kernel/osrelease kernel_osrelease.txt
copy_readable /proc/self/mountinfo self_mountinfo.txt
copy_readable /proc/1/mountinfo init_mountinfo.txt
copy_readable /proc/mounts proc_mounts.txt
copy_readable /sys/fs/selinux/enforce selinux_enforce.txt

if [ -r /proc/cmdline ]; then
  sed -E \
    -e 's/(androidboot\.serialno=)[^ ]+/\1<redacted>/g' \
    -e 's/(androidboot\.boot_devices=)[^ ]+/\1<redacted>/g' \
    /proc/cmdline >"$OUT_DIR/proc_cmdline_redacted.txt" 2>/dev/null || true
fi

capture_sh root_manager_versions.txt '
for command_line in \
  "/data/adb/ksud -V" \
  "/data/adb/ksud --version" \
  "ksud -V" \
  "magisk -V" \
  "magisk -v" \
  "apd --version" \
  "su -V"; do
  echo "## $command_line"
  sh -c "$command_line" 2>&1 || true
  echo
done
'

capture_sh relevant_mounts.txt '
grep -Ei "overlay|tmpfs|loop|hybrid|kasumi|kernelsu|/system|/vendor|/product|/system_ext|/odm|/data/adb" /proc/self/mountinfo 2>/dev/null || true
'

capture_sh loop_devices.txt '
ls -l /dev/block/loop* /dev/loop* 2>/dev/null || true
losetup -a 2>/dev/null || true
'

capture_sh overlay_support.txt '
echo "## /proc/filesystems"
grep -E "(^|[[:space:]])overlay$|tmpfs|ext4" /proc/filesystems 2>/dev/null || true
echo
echo "## overlay module"
ls -la /sys/module/overlay 2>/dev/null || true
echo
echo "## overlay parameters"
find /sys/module/overlay/parameters -maxdepth 1 -type f -print -exec cat {} \; 2>/dev/null || true
'

capture_sh kernel_config_relevant.txt '
if [ -r /proc/config.gz ]; then
  zcat /proc/config.gz 2>/dev/null | grep -E "^CONFIG_(OVERLAY_FS|TMPFS_XATTR|EXT4_FS|MODULES|MODULE_UNLOAD|KPROBES|KALLSYMS|SECURITY_SELINUX|IKCONFIG)=" || true
elif [ -r /boot/config-$(uname -r) ]; then
  grep -E "^CONFIG_(OVERLAY_FS|TMPFS_XATTR|EXT4_FS|MODULES|MODULE_UNLOAD|KPROBES|KALLSYMS|SECURITY_SELINUX|IKCONFIG)=" /boot/config-$(uname -r) || true
else
  echo "kernel config unavailable"
fi
'

capture_sh kasumi_runtime.txt '
echo "## loaded modules"
grep -i kasumi /proc/modules 2>/dev/null || true
echo
echo "## sysfs modules"
find /sys/module -maxdepth 1 -type d -iname "*kasumi*" -print 2>/dev/null || true
echo
echo "## devices"
find /dev -maxdepth 2 -iname "*kasumi*" -o -iname "*hybrid*" 2>/dev/null || true
'

capture_sh module_inventory.txt '
for root in /data/adb/modules /data/adb/modules_update /data/adb/metamodule /data/adb/meta_modules; do
  echo "## $root"
  if [ -d "$root" ]; then
    find "$root" -mindepth 1 -maxdepth 2 \
      \( -name module.prop -o -name disable -o -name remove -o -name skip_mount -o -name update \) \
      -print 2>/dev/null | sort
  else
    echo "not present"
  fi
  echo
done
'

capture_sh rehybird_locations.txt '
find /data/adb -maxdepth 4 \
  \( -type d -o -type f \) \
  \( -iname "*hybrid*" -o -iname "*rehybird*" -o -iname "*kasumi*" \) \
  -print 2>/dev/null | sort
'

for module_root in \
  /data/adb/modules/hybrid_mount \
  /data/adb/modules_update/hybrid_mount \
  /data/adb/metamodule/hybrid_mount \
  /data/adb/meta_modules/hybrid_mount; do
  safe_name="$(printf '%s' "$module_root" | sed 's#^/##; s#[^A-Za-z0-9._-]#_#g')"
  hash_tree "$module_root" "hashes_${safe_name}.txt"
  if [ -d "$module_root" ]; then
    capture_sh "listing_${safe_name}.txt" "ls -laZ '$module_root' 2>/dev/null; find '$module_root' -maxdepth 3 -type f -printf '%p %s bytes\\n' 2>/dev/null | sort"
    [ -r "$module_root/module.prop" ] && cat "$module_root/module.prop" >"$OUT_DIR/module_prop_${safe_name}.txt" 2>/dev/null || true
    [ -r "$module_root/config.toml" ] && cat "$module_root/config.toml" >"$OUT_DIR/config_${safe_name}.toml" 2>/dev/null || true
  fi
done

capture_sh relevant_dmesg.txt '
dmesg 2>/dev/null | grep -Ei "hybrid|rehybird|kasumi|overlay|loop|mount|kernelsu|ksu|avc:" | tail -n 2000 || true
'

capture_sh relevant_logcat.txt '
logcat -b all -d -v threadtime 2>/dev/null | grep -Ei "hybrid|rehybird|kasumi|overlay|loop|mount|kernelsu|ksu|avc:" | tail -n 3000 || true
'

capture_sh data_adb_labels.txt '
ls -ldZ /data/adb /data/adb/modules /data/adb/modules/hybrid_mount 2>/dev/null || true
'

{
  printf 'script_version=%s\n' "$SCRIPT_VERSION"
  printf 'archive=%s\n' "$ARCHIVE"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$0" 2>/dev/null || true
  fi
} >"$OUT_DIR/collector_manifest.txt"

if command -v tar >/dev/null 2>&1; then
  tar -czf "$ARCHIVE" -C "$BASE_DIR" "$(basename "$OUT_DIR")" \
    || fail "failed to create archive"
  chmod 600 "$ARCHIVE" 2>/dev/null || true
else
  fail "tar is unavailable; report remains at $OUT_DIR"
fi

say "Completed without changing mount or module state."
say "Archive: $ARCHIVE"
