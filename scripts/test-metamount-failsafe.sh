#!/usr/bin/env bash
# Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
#
# SPDX-License-Identifier: GPL-3.0-only

set -euo pipefail

temp_dir="$(mktemp -d)"
backup_ksud=""
cleanup() {
  rm -rf "$temp_dir" /data/adb/hybrid-mount
  if [ -n "$backup_ksud" ] && [ -e "$backup_ksud" ]; then
    mkdir -p /data/adb
    mv "$backup_ksud" /data/adb/ksud
  else
    rm -f /data/adb/ksud
  fi
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

echo 'Metamount fail-safe tests passed'
