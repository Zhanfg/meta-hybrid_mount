#!/usr/bin/env bash
# Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
#
# SPDX-License-Identifier: GPL-3.0-only

set -euo pipefail

temp_dir="$(mktemp -d)"
backup_ksud=""
cleanup() {
  rm -rf /data/adb/hybrid-mount
  if [ -n "$backup_ksud" ] && [ -e "$backup_ksud" ]; then
    mkdir -p /data/adb
    mv "$backup_ksud" /data/adb/ksud
  else
    rm -f /data/adb/ksud
  fi
  rm -rf "$temp_dir"
}
trap cleanup EXIT

mkdir -p /data/adb
if [ -e /data/adb/ksud ]; then
  backup_ksud="$temp_dir/original-ksud"
  mv /data/adb/ksud "$backup_ksud"
fi

make_module() {
  module_dir="$1"
  binary_status="$2"
  with_kasumi="$3"
  mkdir -p "$module_dir"
  cp module/metamount.sh "$module_dir/metamount.sh"
  if [ "$with_kasumi" = true ]; then
    mkdir -p "$module_dir/kasumi_lkm"
  fi
  cat >"$module_dir/hybrid-mount" <<EOF
#!/bin/sh
printf '%s\n' "\$*" >>'$module_dir/invocations'
if [ "\${1:-}" = --config ]; then
  exit 0
fi
exit $binary_status
EOF
  chmod 755 "$module_dir/hybrid-mount" "$module_dir/metamount.sh"
}

failure_module="$temp_dir/failure-module"
make_module "$failure_module" 37 true
mkdir -p /data/adb/hybrid-mount/run
printf 'config\n' >/data/adb/hybrid-mount/config.toml
printf stale >/data/adb/hybrid-mount/run/daemon.pid
printf stale >/data/adb/hybrid-mount/run/daemon_state.json

set +e
sh "$failure_module/metamount.sh"
failure_status=$?
set -e

[ "$failure_status" = 37 ]
[ -f "$failure_module/disable" ]
[ -f /data/adb/hybrid-mount/last_boot_failure ]
grep -Fx 'status=37' /data/adb/hybrid-mount/last_boot_failure
[ ! -e /data/adb/hybrid-mount/run/daemon.pid ]
[ ! -e /data/adb/hybrid-mount/run/daemon_state.json ]
grep -Fx -- '--config /data/adb/hybrid-mount/config.toml lkm unload' "$failure_module/invocations"

rm -rf /data/adb/hybrid-mount
success_module="$temp_dir/success-module"
make_module "$success_module" 0 false
mkdir -p /data/adb/hybrid-mount
printf stale >/data/adb/hybrid-mount/last_boot_failure
cat >/data/adb/ksud <<EOF
#!/bin/sh
printf '%s\n' "\$*" >>'$temp_dir/ksud-invocations'
exit 0
EOF
chmod 755 /data/adb/ksud

sh "$success_module/metamount.sh"
[ ! -e "$success_module/disable" ]
[ ! -e /data/adb/hybrid-mount/last_boot_failure ]
grep -Fx 'kernel notify-module-mounted' "$temp_dir/ksud-invocations"

missing_module="$temp_dir/missing-module"
mkdir -p "$missing_module"
cp module/metamount.sh "$missing_module/metamount.sh"
chmod 755 "$missing_module/metamount.sh"
set +e
sh "$missing_module/metamount.sh"
missing_status=$?
set -e
[ "$missing_status" = 1 ]
[ -f "$missing_module/disable" ]

# A symlinked private root must be rejected before stale runtime names are
# touched. The external sentinel proves cleanup did not traverse the link.
rm -rf /data/adb/hybrid-mount
unsafe_root_target="$temp_dir/unsafe-root-target"
mkdir -p "$unsafe_root_target/run"
printf 'must-survive\n' >"$unsafe_root_target/run/daemon.pid"
ln -s "$unsafe_root_target" /data/adb/hybrid-mount
unsafe_root_module="$temp_dir/unsafe-root-module"
make_module "$unsafe_root_module" 0 false
set +e
sh "$unsafe_root_module/metamount.sh" >/dev/null 2>&1
unsafe_root_status=$?
set -e
[ "$unsafe_root_status" = 1 ]
[ -f "$unsafe_root_module/disable" ]
[ "$(cat "$unsafe_root_target/run/daemon.pid")" = must-survive ]
rm -f /data/adb/hybrid-mount

# The same rule applies to a symlinked run directory below an otherwise valid
# private root.
mkdir -p /data/adb/hybrid-mount
chmod 700 /data/adb/hybrid-mount
unsafe_run_target="$temp_dir/unsafe-run-target"
mkdir -p "$unsafe_run_target"
printf 'must-survive\n' >"$unsafe_run_target/daemon_state.json"
ln -s "$unsafe_run_target" /data/adb/hybrid-mount/run
unsafe_run_module="$temp_dir/unsafe-run-module"
make_module "$unsafe_run_module" 0 false
set +e
sh "$unsafe_run_module/metamount.sh" >/dev/null 2>&1
unsafe_run_status=$?
set -e
[ "$unsafe_run_status" = 1 ]
[ -f "$unsafe_run_module/disable" ]
[ "$(cat "$unsafe_run_target/daemon_state.json")" = must-survive ]
rm -f /data/adb/hybrid-mount/run

# The boot entry must not chmod or execute a symlink substituted for the
# packaged binary.
rm -rf /data/adb/hybrid-mount
binary_target="$temp_dir/binary-target"
printf '#!/bin/sh\nexit 0\n' >"$binary_target"
chmod 600 "$binary_target"
symlink_binary_module="$temp_dir/symlink-binary-module"
mkdir -p "$symlink_binary_module"
cp module/metamount.sh "$symlink_binary_module/metamount.sh"
ln -s "$binary_target" "$symlink_binary_module/hybrid-mount"
set +e
sh "$symlink_binary_module/metamount.sh" >/dev/null 2>&1
symlink_binary_status=$?
set -e
[ "$symlink_binary_status" = 1 ]
[ -f "$symlink_binary_module/disable" ]
[ "$(stat -c %a "$binary_target")" = 600 ]

echo 'Metamount fail-safe tests passed'
