# Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
#
# SPDX-License-Identifier: GPL-3.0-only
# shellcheck shell=sh disable=SC3043

if [ -z "${APATCH:-}" ] && [ -z "${KSU:-}" ]; then
  abort "! unsupported root platform"
fi

if [ -n "${KSU_LATE_LOAD:-}" ] && [ -n "${KSU:-}" ]; then
  abort "! unsupported late load mode"
fi

validate_zip_paths() {
  entry_list="${TMPDIR:-/data/local/tmp}/rehybird-zip-entries.$$"
  rm -f "$entry_list"

  if ! unzip -Z1 "$ZIPFILE" >"$entry_list" 2>/dev/null; then
    if ! unzip -l "$ZIPFILE" 2>/dev/null | awk '
      NR <= 3 { next }
      /^[[:space:]]*-+[[:space:]]+-+/ { next }
      {
        line = $0
        sub(/^[[:space:]]*[0-9]+[[:space:]]+[0-9-]+[[:space:]]+[0-9:]+[[:space:]]+/, "", line)
        if (line != "") print line
      }
    ' >"$entry_list"; then
      rm -f "$entry_list"
      return 1
    fi
  fi

  unsafe=false
  while IFS= read -r entry; do
    case "$entry" in
    /* | ../* | */../* | */.. | *\\*)
      ui_print "! Unsafe archive entry: $entry"
      unsafe=true
      break
      ;;
    esac
  done <"$entry_list"
  rm -f "$entry_list"
  [ "$unsafe" = false ]
}

ui_print "- Verifying archive CRC..."
if ! unzip -t "$ZIPFILE" >/dev/null 2>&1; then
  abort "! Package CRC verification failed; refusing partial installation"
fi
if ! validate_zip_paths; then
  abort "! Package contains an unsafe path"
fi
if ! unzip -o "$ZIPFILE" -d "$MODPATH" >&2; then
  abort "! Package extraction failed; refusing partial installation"
fi

if [ ! -r "$MODPATH/package-integrity.sh" ]; then
  abort "! Package integrity validator is missing"
fi
# shellcheck source=module/package-integrity.sh
. "$MODPATH/package-integrity.sh"
if ! rehybird_validate_package_tree "$MODPATH"; then
  abort "! Package structure or flavor validation failed"
fi
ui_print "- Package integrity verified: $REHYBIRD_PACKAGE_FLAVOR"

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
