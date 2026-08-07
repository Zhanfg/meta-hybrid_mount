# Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
#
# SPDX-License-Identifier: GPL-3.0-only

MODDIR="${0%/*}"
BASE_DIR="/data/adb/hybrid-mount"
RUN_DIR="$BASE_DIR/run"
PID_FILE="$RUN_DIR/daemon.pid"
SOCKET_FILE="$RUN_DIR/daemon.sock"
STATE_FILE="$RUN_DIR/daemon_state.json"
KASUMI_RULE_SNAPSHOT_FILE="$RUN_DIR/kasumi_mount_rules.json"

mkdir -p "$BASE_DIR" "$RUN_DIR"

BINARY="$MODDIR/hybrid-mount"

if [ ! -f "$BINARY" ]; then
  echo "ERROR: Binary not found at $BINARY"
  exit 1
fi

cleanup_runtime_files() {
  rm -f \
    "$PID_FILE" \
    "$SOCKET_FILE" \
    "$STATE_FILE" \
    "$KASUMI_RULE_SNAPSHOT_FILE"
}

cleanup_empty_staging_dirs() {
  for stale_dir in "$RUN_DIR"/staging_*; do
    [ -d "$stale_dir" ] || continue
    rmdir "$stale_dir" 2>/dev/null || true
  done
}

chmod 755 "$BINARY"
cleanup_runtime_files
cleanup_empty_staging_dirs

"$BINARY"
STATUS=$?

if [ "$STATUS" -eq 0 ] && [ -x /data/adb/ksud ]; then
  /data/adb/ksud kernel notify-module-mounted
fi

exit "$STATUS"
