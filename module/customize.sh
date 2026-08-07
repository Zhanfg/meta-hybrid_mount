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
MODULE_BACKUP="$INSTALL_WORK_ROOT/previous-module-tree"
PRESERVE_INSTALL_WORK=false
CONFIG_CREATED=false
BLACKLIST_CREATED=false
PRIVATE_ROOT_CREATED=false
BASE_DIR=""
CONFIG_PATH=""
BLACKLIST_PATH=""

cleanup_install_work() {
  if [ "${PRESERVE_INSTALL_WORK:-false}" = true ]; then
    echo "REHYBIRD: preserving recovery workspace at $INSTALL_WORK_ROOT" >&2
    return 0
  fi
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
      if (name ~ /^\// || name ~ /^[A-Za-z]:/ || name ~ /(^|\/)\.\.(\/|$)/ || name ~ /(^|\/)\.(\/|$)/ || name ~ /\/\// || name ~ /\\/) bad = 1
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

cleanup_created_private_data() {
  cleanup_failed=false

  if [ "${CONFIG_CREATED:-false}" = true ] && [ -n "${CONFIG_PATH:-}" ]; then
    rm -f "$CONFIG_PATH" || cleanup_failed=true
    CONFIG_CREATED=false
  fi
  if [ "${BLACKLIST_CREATED:-false}" = true ] && [ -n "${BLACKLIST_PATH:-}" ]; then
    rm -f "$BLACKLIST_PATH" || cleanup_failed=true
    BLACKLIST_CREATED=false
  fi
  if [ "${PRIVATE_ROOT_CREATED:-false}" = true ] && [ -n "${BASE_DIR:-}" ]; then
    rmdir "$BASE_DIR" 2>/dev/null || true
    PRIVATE_ROOT_CREATED=false
  fi

  [ "$cleanup_failed" = false ]
}

rollback_install_or_preserve() {
  reason="$1"
  private_cleanup_ok=true
  cleanup_created_private_data || private_cleanup_ok=false

  if rehybird_rollback_module_tree "$MODPATH" "$MODULE_BACKUP"; then
    if [ "$private_cleanup_ok" = true ]; then
      abort "$reason; module tree was rolled back"
    fi
    abort "$reason; module tree was rolled back but new private-data cleanup was incomplete"
  fi

  PRESERVE_INSTALL_WORK=true
  abort "$reason; rollback failed, recovery workspace preserved at $INSTALL_WORK_ROOT"
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
if [ ! -r "$STAGED_TREE/package-integrity.sh" ] || ! sh -n "$STAGED_TREE/package-integrity.sh"; then
  abort "! Staged package integrity validator is missing or invalid"
fi
# shellcheck source=module/package-integrity.sh
. "$STAGED_TREE/package-integrity.sh"
if ! rehybird_validate_package_tree "$STAGED_TREE"; then
  abort "! Staged package structure or flavor validation failed"
fi
STAGED_FLAVOR="$REHYBIRD_PACKAGE_FLAVOR"

case "$ARCH" in
"arm64")
  ;;
*)
  abort "! Unsupported architecture: $ARCH (Hybrid Mount now supports arm64 only)"
  ;;
esac
ui_print "- Device Architecture: $ARCH"

stage_status=0
rehybird_stage_module_tree "$STAGED_TREE" "$MODPATH" "$MODULE_BACKUP" || stage_status=$?
case "$stage_status" in
0)
  ;;
3)
  PRESERVE_INSTALL_WORK=true
  abort "! Verified package staging and rollback both failed; recovery workspace preserved at $INSTALL_WORK_ROOT"
  ;;
*)
  abort "! Verified package could not replace installer staging tree"
  ;;
esac

if [ ! -r "$MODPATH/package-integrity.sh" ] || ! sh -n "$MODPATH/package-integrity.sh"; then
  rollback_install_or_preserve "! Final package integrity validator is missing or invalid"
fi
# shellcheck source=module/package-integrity.sh
. "$MODPATH/package-integrity.sh"
if ! rehybird_validate_package_tree "$MODPATH"; then
  rollback_install_or_preserve "! Final package tree failed post-copy validation"
fi
if [ "$REHYBIRD_PACKAGE_FLAVOR" != "$STAGED_FLAVOR" ]; then
  rollback_install_or_preserve "! Package flavor changed during installation"
fi
ui_print "- Package integrity verified: $REHYBIRD_PACKAGE_FLAVOR"

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
  rollback_install_or_preserve "! Failed to install verified binary"
fi
if ! set_perm "$BIN_TARGET" 0 0 0755; then
  rollback_install_or_preserve "! Failed to set binary permissions"
fi
if ! rm -rf "$MODPATH/binaries" "$MODPATH/system"; then
  rollback_install_or_preserve "! Failed to remove package staging payloads"
fi
if [ "$NANO_MODE" = true ]; then
  if ! rm -rf "$MODPATH/webroot" "$MODPATH/launcher.png"; then
    rollback_install_or_preserve "! Failed to remove Nano-incompatible WebUI payloads"
  fi
fi

if [ ! -r "$MODPATH/metasafety.sh" ] || ! sh -n "$MODPATH/metasafety.sh"; then
  rollback_install_or_preserve "! Private data safety helper is missing or invalid"
fi
# shellcheck source=module/metasafety.sh
. "$MODPATH/metasafety.sh"

BASE_DIR="/data/adb/hybrid-mount"
CONFIG_PATH="$BASE_DIR/config.toml"
BLACKLIST_PATH="$BASE_DIR/module_blacklist.toml"
PRIVATE_ROOT_PREEXISTED=false
if [ -e "$BASE_DIR" ] || [ -L "$BASE_DIR" ]; then
  PRIVATE_ROOT_PREEXISTED=true
fi
if ! rehybird_prepare_private_root "$BASE_DIR"; then
  rollback_install_or_preserve "! Private data root is unsafe; refusing installation"
fi
if [ "$PRIVATE_ROOT_PREEXISTED" = false ]; then
  PRIVATE_ROOT_CREATED=true
fi

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
  if ! sed -i "s/^default_mode = .*/default_mode = \"$chosen_mode\"/" "$CONFIG_PATH"; then
    rollback_install_or_preserve "! Failed to update default mount mode"
  fi
  if ! grep -Fx "default_mode = \"$chosen_mode\"" "$CONFIG_PATH" >/dev/null 2>&1; then
    rollback_install_or_preserve "! Default mount mode update was not applied"
  fi
  if ! rehybird_secure_existing_private_file "$CONFIG_PATH"; then
    rollback_install_or_preserve "! Updated config file failed private-file validation"
  fi
}

if [ -e "$CONFIG_PATH" ] || [ -L "$CONFIG_PATH" ]; then
  if ! rehybird_secure_existing_private_file "$CONFIG_PATH"; then
    rollback_install_or_preserve "! Existing config is not a trusted root-owned regular file"
  fi
  ui_print "- Existing config found"
  ui_print "- Skipping setup wizard to preserve settings"
else
  ui_print "- Fresh installation detected"
  ui_print "- Installing default config atomically..."
  if ! rehybird_install_private_file "$MODPATH/config.toml" "$CONFIG_PATH"; then
    rollback_install_or_preserve "! Failed to install trusted default config"
  fi
  CONFIG_CREATED=true
  if [ "$NANO_MODE" = true ]; then
    ui_print "- Nano mode uses config.toml only; skipping setup wizard"
  else
    KEY_volume_detect
  fi
fi

if [ -e "$BLACKLIST_PATH" ] || [ -L "$BLACKLIST_PATH" ]; then
  if ! rehybird_secure_existing_private_file "$BLACKLIST_PATH"; then
    rollback_install_or_preserve "! Existing module blacklist is not a trusted root-owned regular file"
  fi
else
  ui_print "- Installing default module blacklist atomically..."
  if ! rehybird_install_private_file "$MODPATH/module_blacklist.toml" "$BLACKLIST_PATH"; then
    rollback_install_or_preserve "! Failed to install trusted module blacklist"
  fi
  BLACKLIST_CREATED=true
fi

if ! set_perm_recursive "$MODPATH" 0 0 0755 0644; then
  rollback_install_or_preserve "! Failed to set module permissions"
fi
if ! set_perm "$BIN_TARGET" 0 0 0755; then
  rollback_install_or_preserve "! Failed to finalize binary permissions"
fi

if ! rehybird_commit_module_tree "$MODULE_BACKUP"; then
  PRESERVE_INSTALL_WORK=true
  ui_print "! Installation succeeded, but the previous module-tree backup could not be removed"
  ui_print "! Recovery workspace preserved: $INSTALL_WORK_ROOT"
else
  cleanup_install_work
  trap - 0 1 2 15
fi

ui_print "- Installation complete"
