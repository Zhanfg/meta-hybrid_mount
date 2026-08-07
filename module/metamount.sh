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
BINARY="$MODDIR/hybrid-mount"

self_disable_without_runtime_touch() {
  reason="$1"
  rm -f "$DISABLE_FILE"
  if ! : >"$DISABLE_FILE"; then
    echo "ERROR: $reason; self-disable marker could not be created" >&2
  else
    echo "ERROR: $reason; REHYBIRD disabled for next boot" >&2
  fi
  exit 1
}

secure_runtime_dir() {
  path="$1"

  if [ -L "$path" ]; then
    return 1
  fi
  if [ -e "$path" ] && [ ! -d "$path" ]; then
    return 1
  fi
  if [ ! -d "$path" ]; then
    mkdir -m 0700 "$path" || return 1
  fi
  [ -d "$path" ] && [ ! -L "$path" ] || return 1
  chown 0:0 "$path" 2>/dev/null || return 1
  chmod 0700 "$path" 2>/dev/null || return 1

  resolved="$(readlink -f "$path" 2>/dev/null || true)"
  [ "$resolved" = "$path" ] || return 1
}

# Fail closed before creating, deleting or overwriting anything below the
# private runtime tree. Rust repeats these checks, but shell startup must not
# follow an unsafe path before the Rust process gets a chance to reject it.
if [ -L "$BASE_DIR" ] || { [ -e "$BASE_DIR" ] && [ ! -d "$BASE_DIR" ]; }; then
  self_disable_without_runtime_touch "private data root is not a real directory"
fi
if ! secure_runtime_dir "$BASE_DIR"; then
  self_disable_without_runtime_touch "private data root failed ownership/path validation"
fi
if [ -L "$RUN_DIR" ] || { [ -e "$RUN_DIR" ] && [ ! -d "$RUN_DIR" ]; }; then
  self_disable_without_runtime_touch "runtime directory is not a real directory"
fi
if ! secure_runtime_dir "$RUN_DIR"; then
  self_disable_without_runtime_touch "runtime directory failed ownership/path validation"
fi

if [ ! -f "$BINARY" ] || [ -L "$BINARY" ]; then
  self_disable_without_runtime_touch "binary is missing or is a symbolic link: $BINARY"
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

chmod 755 "$BINARY" || self_disable_without_runtime_touch "binary permissions could not be secured"
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
