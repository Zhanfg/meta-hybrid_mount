#!/usr/bin/env bash
# Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
#
# SPDX-License-Identifier: GPL-3.0-only

set -euo pipefail

# shellcheck source=module/metasafety.sh
. module/metasafety.sh

for valid in alpha MyModule a1_b.c-d; do
  rehybird_valid_module_id "$valid" || {
    echo "valid module ID rejected: $valid" >&2
    exit 1
  }
done
for invalid in '' a 1alpha -alpha ../escape 'alpha/beta' 'alpha beta'; do
  if rehybird_valid_module_id "$invalid"; then
    echo "invalid module ID accepted: $invalid" >&2
    exit 1
  fi
done

temp_dir="$(mktemp -d)"
cleanup() {
  rm -rf "$temp_dir"
  rm -rf /data/adb/hybrid-mount
}
trap cleanup EXIT

active="$temp_dir/modules/alpha"
update="$temp_dir/modules_update/alpha"
backup="$temp_dir/modules/alpha.rehybird-backup"
mkdir -p "$active" "$update"
printf 'old\n' >"$active/version"
printf 'new\n' >"$update/version"
printf 'id=alpha\n' >"$active/module.prop"
printf 'id=alpha\n' >"$update/module.prop"

rehybird_stage_module_replace "$active" "$update" "$backup"
test "$(cat "$active/version")" = new
test "$(cat "$backup/version")" = old
test ! -e "$update"

rehybird_rollback_module_replace "$active" "$update" "$backup"
test "$(cat "$active/version")" = old
test "$(cat "$update/version")" = new
test ! -e "$backup"

rehybird_stage_module_replace "$active" "$update" "$backup"
mkdir -p "$update"
cat "$active/module.prop" >"$update/module.prop"
rehybird_commit_module_replace "$backup"
test "$(cat "$active/version")" = new
test -f "$update/module.prop"
test ! -e "$backup"

mkdir -p "$temp_dir/missing-active"
printf 'preserve\n' >"$temp_dir/missing-active/version"
if rehybird_stage_module_replace \
  "$temp_dir/missing-active" \
  "$temp_dir/missing-update" \
  "$temp_dir/missing-backup"; then
  echo 'transaction unexpectedly succeeded without update directory' >&2
  exit 1
fi
test "$(cat "$temp_dir/missing-active/version")" = preserve
test ! -e "$temp_dir/missing-backup"

mkdir -p "$temp_dir/bin"
cat >"$temp_dir/bin/mountpoint" <<'EOF'
#!/bin/sh
exit 0
EOF
chmod 755 "$temp_dir/bin/mountpoint"

mkdir -p /data/adb/hybrid-mount/mnt
mkdir -p /data/adb/hybrid-mount/sentinel
printf 'keep\n' >/data/adb/hybrid-mount/sentinel/value
PATH="$temp_dir/bin:$PATH" MODULE_ID='../sentinel' sh module/metauninstall.sh

test -f /data/adb/hybrid-mount/sentinel/value

mkdir -p /data/adb/hybrid-mount/mnt/alpha
printf 'remove\n' >/data/adb/hybrid-mount/mnt/alpha/value
PATH="$temp_dir/bin:$PATH" MODULE_ID='alpha' sh module/metauninstall.sh

test ! -e /data/adb/hybrid-mount/mnt/alpha

echo 'Metamodule safety tests passed'
