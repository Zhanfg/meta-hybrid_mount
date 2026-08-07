# Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
#
# SPDX-License-Identifier: GPL-3.0-only
# shellcheck shell=sh disable=SC3043

umask 077

if [ -z "${APATCH:-}" ] && [ -z "${KSU:-}" ]; then
  abort "! unsupported root platform"
fi

if [ -n "${KSU_LATE_LOAD:-}" ] && [ -n "${KSU:-}" ]; then
  abort "! unsupported late load mode"
fi

create_private_work_root() {
  [ -d /data/local/tmp ] && [ ! -L /data/local/tmp ] || return 1
  if command -v mktemp >/dev/null 2>&1; then
    mktemp -d /data/local/tmp/rehybird-install.XXXXXX
  elif command -v busybox >/dev/null 2>&1; then
    busybox mktemp -d /data/local/tmp/rehybird-install.XXXXXX
  else
    return 1
  fi
}

INSTALL_WORK_ROOT="$(create_private_work_root)" || abort "! Failed to create random private installation workspace"
BOOTSTRAP_LISTING="$INSTALL_WORK_ROOT/archive.listing"
ARCHIVE_HELPER="$INSTALL_WORK_ROOT/archive-safety.sh"
ARCHIVE_MANIFEST="$INSTALL_WORK_ROOT/archive.manifest"
STAGED_TREE="$INSTALL_WORK_ROOT/staged"

cleanup_install_work() {
  rm -rf "$INSTALL_WORK_ROOT"
}
trap cleanup_install_work 0
trap 'cleanup_install_work; exit 1' 1 2 15

bootstrap_validate_archive() {
  rm -f "$BOOTSTRAP_LISTING"
  unzip -l "$ZIPFILE" >"$BOOTSTRAP_LISTING" 2>/dev/null || return 1

  awk -v max_file=33554432 -v max_total=100663296 -v max_entries=5000 '
    $1 ~ /^[0-9]+$/ && $2 ~ /^[0-9-]+$/ && $3 ~ /^[0-9:]+$/ {
      size = $1 + 0
      name = $0
      sub(/^[[:space:]]*[0-9]+[[:space:]]+[0-9-]+[[:space:]]+[0-9:]+[[:space:]]+/, "", name)
      if (name == "") next

      count++
      total += size
      if (size > max_file || total > max_total || count > max_entries) bad = 1
      if (seen[name]++) bad = 1
      if (name !~ /^[A-Za-z0-9._\/-]+$/) bad = 1
      if (name ~ /^\// || name ~ /^[A-Za-z]:/ || name ~ /(^|\/)\.\.(\/|$)/ || name ~ /\\/) bad = 1
      if (name == "archive-safety.sh") helper_count++
    }
    END {
      if (count == 0 || bad || helper_count != 1) exit 1
    }
  ' "$BOOTSTRAP_LISTING"
  status=$?
  rm -f "$BOOTSTRAP_LISTING"
  return "$status"
}

ui_print "- Verifying archive CRC and bootstrap layout..."
if ! unzip -t "$ZIPFILE" >/dev/null 2>&1; then
  abort "! Package CRC verification failed; refusing partial installation"
fi
chmod 700 "$INSTALL_WORK_ROOT" 2>/dev/null || true
if ! bootstrap_validate_archive; then
  abort "! Package archive layout, size, name, or duplicate-entry validation failed"
fi
if ! unzip -p "$ZIPFILE" archive-safety.sh >"$ARCHIVE_HELPER"; then
  abort "! Failed to read archive safety helper"
fi
chmod 600 "$ARCHIVE_HELPER" 2>/dev/null || true
if [ ! -s "$ARCHIVE_HELPER" ] || ! sh -n "$ARCHIVE_HELPER"; then
  abort "! Archive safety helper is empty or invalid"
fi
# shellcheck source=module/archive-safety.sh
. "$ARCHIVE_HELPER"

if ! rehybird_build_archive_manifest "$ZIPFILE" "$ARCHIVE_MANIFEST"; then
  abort "! Package manifest validation failed"
fi
if ! rehybird_extract_regular_archive "$ZIPFILE" "$STAGED_TREE" "$ARCHIVE_MANIFEST"; then
  abort "! Regular-file-only package extraction failed"
fi
if [ ! -r "$STAGED_TREE/package-integrity.sh" ]; then
  abort "! Package integrity validator is missing from staged tree"
fi
# shellcheck source=module/package-integrity.sh
. "$STAGED_TREE/package-integrity.sh"
if ! rehybird_validate_package_tree "$STAGED_TREE"; then
  abort "! Staged package structure or flavor validation failed"
fi
STAGED_FLAVOR="$REHYBIRD_PACKAGE_FLAVOR"

replace_status=0
rehybird_replace_module_tree "$STAGED_TREE" "$MODPATH" || replace_status=$?
case "$replace_status" in
0)
  ;;
2)
  abort "! New package backup cleanup failed; previous module tree was restored"
  ;;
3)
  abort "! Verified package installation and rollback both failed"
  ;;
*)
  abort "! Verified package could not replace installer staging tree"
  ;;
esac

if [ ! -r "$MODPATH/package-integrity.sh" ]; then
  abort "! Final package integrity validator is missing"
fi
# shellcheck source=module/package-integrity.sh
. "$MODPATH/package-integrity.sh"
if ! rehybird_validate_package_tree "$MODPATH"; then
  abort "! Final package tree failed post-copy validation"
fi
if [ "$REHYBIRD_PACKAGE_FLAVOR" != "$STAGED_FLAVOR" ]; then
  abort "! Package flavor changed during installation"
fi
ui_print "- Package integrity verified: $REHYBIRD_PACKAGE_FLAVOR"

cleanup_install_work
trap - 0 1 2 15

case "$ARCH" in
"arm64")
  ;;
*)
  abort "! Unsupported architecture: $ARCH (Hybrid Mount now supports arm64 only)"
  ;;
esac
ui_print "- Device Architecture: $ARCH"

NANO_MODE=false
if [ "$REHYBIRD_PACKAGE_FLAVOR" = nano ]; then
  NANO_MODE=true
  ui_print "- Flavor: Nano (config-only)"
elif [ "$REHYBIRD_PACKAGE_FLAVOR" = lite ]; then
  ui_print "- Flavor: Lite"
else
  ui_print "- Flavor: Full"
fi

BIN_SOURCE="$MODPATH/binaries/hybrid-mount"
BIN_TARGET="$MODPATH/hybrid-mount"
ui_print "- Installing binary..."
if ! cp -f "$BIN_SOURCE" "$BIN_TARGET"; then
  abort "! Failed to install verified binary"
fi
set_perm "$BIN_TARGET" 0 0 0755
rm -rf "$MODPATH/binaries"
rm -rf "$MODPATH/system"
if [ "$NANO_MODE" = "true" ]; then
  rm -rf "$MODPATH/webroot" "$MODPATH/launcher.png"
fi
BASE_DIR="/data/adb/hybrid-mount"
mkdir -p "$BASE_DIR" || abort "! Failed to create $BASE_DIR"

wait_volume_key_or_timeout() {
  local timeout_seconds start_time current_time key_event
  timeout_seconds=$1
  start_time=$(date +%s)
  while true; do
    current_time=$(date +%s)
    if [ $((current_time - start_time)) -ge "$timeout_seconds" ]; then
      printf 'timeout\n'
      return 0
    fi
    key_event=$(timeout 0.5 getevent -l 2>/dev/null || true)
    if echo "$key_event" | grep -q "KEY_VOLUMEUP"; then
      printf 'up\n'
      return 0
    elif echo "$key_event" | grep -q "KEY_VOLUMEDOWN"; then
      printf 'down\n'
      return 0
    fi
    sleep 0.1
  done
}

KEY_volume_detect() {
  ui_print " "
  ui_print "========================================"
  ui_print "      Select Default Mount Mode      "
  ui_print "========================================"
  ui_print "  Volume Up (+): OverlayFS"
  ui_print "  Volume Down (-): Magic Mount"
  ui_print " "
  ui_print "  Defaulting to OverlayFS in 10 seconds"
  ui_print "========================================"
  local timeout=10
  local chosen_mode="overlay"
  case "$(wait_volume_key_or_timeout "$timeout")" in
  up)
    chosen_mode="overlay"
    ui_print "- Key Detected: Selected OverlayFS"
    ;;
  down)
    chosen_mode="magic"
    ui_print "- Key Detected: Selected Magic Mount"
    ;;
  timeout)
    ui_print "- Timeout: Selected OverlayFS"
    ;;
  esac
  ui_print "- Configured mode: $chosen_mode"
  sed -i "s/^default_mode = .*/default_mode = \"$chosen_mode\"/" "$BASE_DIR/config.toml"
}

if [ ! -f "$BASE_DIR/config.toml" ]; then
  ui_print "- Fresh installation detected"
  ui_print "- Installing default config..."
  if ! cat "$MODPATH/config.toml" >"$BASE_DIR/config.toml"; then
    abort "! Failed to install default config"
  fi
  if [ "$NANO_MODE" = "true" ]; then
    ui_print "- Nano mode uses config.toml only; skipping setup wizard"
  else
    KEY_volume_detect
  fi
else
  ui_print "- Existing config found"
  ui_print "- Skipping setup wizard to preserve settings"
fi

if [ ! -f "$BASE_DIR/module_blacklist.toml" ]; then
  ui_print "- Installing default module blacklist..."
  if ! cat "$MODPATH/module_blacklist.toml" >"$BASE_DIR/module_blacklist.toml"; then
    abort "! Failed to install default module blacklist"
  fi
fi

set_perm_recursive "$MODPATH" 0 0 0755 0644
set_perm "$BIN_TARGET" 0 0 0755
ui_print "- Installation complete"
