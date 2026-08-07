# Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
#
# SPDX-License-Identifier: GPL-3.0-only

umask 077
MODDIR="${0%/*}"
BASE_DIR="/data/adb/hybrid-mount"
RUN_DIR="$BASE_DIR/run"
PID_FILE="$RUN_DIR/daemon.pid"
SOCKET_FILE="$RUN_DIR/daemon.sock"
STATE_FILE="$RUN_DIR/daemon_state.json"
KASUMI_RULE_SNAPSHOT_FILE="$RUN_DIR/kasumi_mount_rules.json"
FAILURE_FILE="$BASE_DIR/last_boot_failure"
DISABLE_FILE="$MODDIR/disable"

mkdir -p "$BASE_DIR" "$RUN_DIR" || exit 1

BINARY="$MODDIR/hybrid-mount"

if [ ! -f "$BINARY" ]; then
  echo "ERROR: Binary not found at $BINARY"
  rm -f "$DISABLE_FILE"
  : >"$DISABLE_FILE"
  exit 1
fi

cleanup_runtime_files() {
  rm -f \
    "$PID_FILE" \
    "$SOCKET_FILE" \
    "$STATE_FILE" \
    "$KASUMI_RULE_SNAPSHOT_FILE"
}

record_boot_failure() {
  status="$1"
  temporary="$FAILURE_FILE.tmp.$$"
  {
    printf 'status=%s\n' "$status"
    printf 'time=%s\n' "$(date +%s 2>/dev/null || echo unknown)"
    printf 'action=module_disabled_for_next_boot\n'
  } >"$temporary" 2>/dev/null || return 1
  chmod 600 "$temporary" 2>/dev/null || true
  mv -f "$temporary" "$FAILURE_FILE"
}

best_effort_unload_owned_kasumi() {
  # Only Full packages ship Kasumi assets. The Rust command checks the
  # current-boot ownership receipt and refuses to unload an unknown module.
  if [ -d "$MODDIR/kasumi_lkm" ] && [ -f "$BASE_DIR/config.toml" ]; then
    "$BINARY" --config "$BASE_DIR/config.toml" lkm unload >/dev/null 2>&1 || true
  fi
}

chmod 755 "$BINARY"
cleanup_runtime_files

"$BINARY"
STATUS=$?

if [ "$STATUS" -eq 0 ]; then
  rm -f "$FAILURE_FILE"
  if [ -x /data/adb/ksud ]; then
    /data/adb/ksud kernel notify-module-mounted >/dev/null 2>&1 || true
  fi
  exit 0
fi

best_effort_unload_owned_kasumi
cleanup_runtime_files
record_boot_failure "$STATUS" || true
rm -f "$DISABLE_FILE"
if ! : >"$DISABLE_FILE"; then
  echo "ERROR: mount failed with status $STATUS and self-disable marker could not be created" >&2
else
  echo "ERROR: mount failed with status $STATUS; REHYBIRD disabled for next boot" >&2
fi
exit "$STATUS"
