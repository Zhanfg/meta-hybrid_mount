#!/system/bin/sh
# Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
#
# SPDX-License-Identifier: GPL-3.0-only

############################################
# Hybrid Mount uninstall.sh
# Cleanup script for metamodule removal
############################################

MODDIR="${0%/*}"
if [ "$MODDIR" = "$0" ]; then
  MODDIR="$(pwd)"
fi
BASE_DIR="/data/adb/hybrid-mount"
CONFIG_FILE="$BASE_DIR/config.toml"
PID_FILE="$BASE_DIR/run/daemon.pid"
SOCKET_FILE="$BASE_DIR/run/daemon.sock"
BINARY="$MODDIR/hybrid-mount"

if [ -x "$BINARY" ] && [ -f "$CONFIG_FILE" ]; then
  # LKM unload is a daemon command. Run it before shutdown so the daemon can
  # clear Kasumi state, release its cached client FD, and delete only an LKM
  # covered by the current-boot ownership receipt. Lite and Nano reject this
  # command, so cleanup remains best-effort for those flavors.
  "$BINARY" --config "$CONFIG_FILE" lkm unload >/dev/null 2>&1 || true

  if [ -S "$SOCKET_FILE" ] || [ -r "$PID_FILE" ]; then
    daemon_pid=""
    if [ -r "$PID_FILE" ]; then
      daemon_pid=$(cat "$PID_FILE" 2>/dev/null || true)
    fi

    "$BINARY" --config "$CONFIG_FILE" daemon stop >/dev/null 2>&1 || true

    case "$daemon_pid" in
    '' | *[!0-9]*)
      ;;
    *)
      wait_count=0
      while kill -0 "$daemon_pid" 2>/dev/null && [ "$wait_count" -lt 20 ]; do
        sleep 0.1
        wait_count=$((wait_count + 1))
      done
      ;;
    esac
  fi
fi

rm -rf "$BASE_DIR"

exit 0
