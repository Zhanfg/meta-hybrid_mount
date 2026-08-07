#!/usr/bin/env bash
# Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
#
# SPDX-License-Identifier: GPL-3.0-only

set -euo pipefail

# shellcheck source=module/uninstall-safety.sh
. module/uninstall-safety.sh

temp_dir="$(mktemp -d)"
trap 'rm -rf "$temp_dir"' EXIT

self_mountinfo="$temp_dir/self.mountinfo"
init_mountinfo="$temp_dir/init.mountinfo"
: >"$self_mountinfo"
: >"$init_mountinfo"

active_base="$temp_dir/hybrid-mount"
mkdir -p "$active_base/rw"
printf 'keep\n' >"$active_base/rw/value"
printf '42 31 0:1 / /system rw - overlay overlay rw,lowerdir=/system,upperdir=%s/rw/system,workdir=%s/work/system\n' \
  "$active_base" "$active_base" >"$self_mountinfo"

set +e
rehybird_remove_inactive_base "$active_base" "$self_mountinfo" "$init_mountinfo"
active_status=$?
set -e
[ "$active_status" = 10 ]
[ -f "$active_base/rw/value" ]

init_active="$temp_dir/init-active"
mkdir -p "$init_active/mnt"
printf 'keep\n' >"$init_active/mnt/value"
: >"$self_mountinfo"
printf '43 31 0:2 / %s/mnt rw - ext4 /dev/loop0 rw\n' "$init_active" >"$init_mountinfo"
set +e
rehybird_remove_inactive_base "$init_active" "$self_mountinfo" "$init_mountinfo"
init_status=$?
set -e
[ "$init_status" = 10 ]
[ -f "$init_active/mnt/value" ]

inactive_base="$temp_dir/inactive"
mkdir -p "$inactive_base"
printf 'remove\n' >"$inactive_base/value"
: >"$self_mountinfo"
: >"$init_mountinfo"
rehybird_remove_inactive_base "$inactive_base" "$self_mountinfo" "$init_mountinfo"
[ ! -e "$inactive_base" ]

similar_base="$temp_dir/hybrid-mount"
similar_other="$temp_dir/hybrid-mount-old"
mkdir -p "$similar_base" "$similar_other"
printf 'remove\n' >"$similar_base/value"
printf '44 31 0:3 / %s rw - tmpfs tmpfs rw\n' "$similar_other" >"$self_mountinfo"
rehybird_remove_inactive_base "$similar_base" "$self_mountinfo" "$init_mountinfo"
[ ! -e "$similar_base" ]
[ -d "$similar_other" ]

symlink_target="$temp_dir/symlink-target"
symlink_base="$temp_dir/symlink-base"
mkdir -p "$symlink_target"
printf 'keep\n' >"$symlink_target/value"
ln -s "$symlink_target" "$symlink_base"
: >"$self_mountinfo"
rehybird_remove_inactive_base "$symlink_base" "$self_mountinfo" "$init_mountinfo"
[ ! -L "$symlink_base" ]
[ -f "$symlink_target/value" ]

echo 'Uninstall safety tests passed'
