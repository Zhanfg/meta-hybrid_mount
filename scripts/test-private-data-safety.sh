#!/usr/bin/env bash
# Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
#
# SPDX-License-Identifier: GPL-3.0-only

set -euo pipefail

# shellcheck source=module/metasafety.sh
. module/metasafety.sh

temp_dir="$(mktemp -d)"
backup_root=""
cleanup() {
  rm -rf /data/adb/hybrid-mount
  if [ -n "$backup_root" ] && [ -e "$backup_root" ]; then
    mv "$backup_root" /data/adb/hybrid-mount
  fi
  rm -rf "$temp_dir"
}
trap cleanup EXIT

mkdir -p /data/adb
if [ -e /data/adb/hybrid-mount ] || [ -L /data/adb/hybrid-mount ]; then
  backup_root="$temp_dir/original-hybrid-mount"
  mv /data/adb/hybrid-mount "$backup_root"
fi

rehybird_prepare_private_root /data/adb/hybrid-mount
[ -d /data/adb/hybrid-mount ]
[ ! -L /data/adb/hybrid-mount ]
[ "$(rehybird_stat_value %u /data/adb/hybrid-mount)" = 0 ]
[ "$(rehybird_stat_value %a /data/adb/hybrid-mount)" = 700 ]

source_file="$temp_dir/default.toml"
target_file="/data/adb/hybrid-mount/config.toml"
printf 'default_mode = "overlay"\n' >"$source_file"
rehybird_install_private_file "$source_file" "$target_file"
[ -f "$target_file" ]
[ ! -L "$target_file" ]
[ "$(rehybird_stat_value %u "$target_file")" = 0 ]
[ "$(rehybird_stat_value %h "$target_file")" = 1 ]
[ "$(rehybird_stat_value %a "$target_file")" = 600 ]
[ "$(cat "$target_file")" = 'default_mode = "overlay"' ]

chmod 666 "$target_file"
rehybird_secure_existing_private_file "$target_file"
[ "$(rehybird_stat_value %a "$target_file")" = 600 ]

rm -f "$target_file"
symlink_target="$temp_dir/symlink-target.toml"
printf 'unsafe\n' >"$symlink_target"
ln -s "$symlink_target" "$target_file"
if rehybird_secure_existing_private_file "$target_file"; then
  echo 'private-file symlink was accepted' >&2
  exit 1
fi
[ "$(cat "$symlink_target")" = unsafe ]
rm -f "$target_file"

printf 'hardlinked\n' >"$target_file"
hardlink_peer="$temp_dir/hardlink-peer.toml"
ln "$target_file" "$hardlink_peer"
if rehybird_secure_existing_private_file "$target_file"; then
  echo 'hard-linked private file was accepted' >&2
  exit 1
fi
rm -f "$target_file" "$hardlink_peer"

printf 'wrong-owner\n' >"$target_file"
chown 65534:65534 "$target_file"
if rehybird_secure_existing_private_file "$target_file"; then
  echo 'non-root-owned private file was accepted' >&2
  exit 1
fi
rm -f "$target_file"

rm -rf /data/adb/hybrid-mount
link_root_target="$temp_dir/link-root-target"
mkdir -p "$link_root_target"
ln -s "$link_root_target" /data/adb/hybrid-mount
if rehybird_prepare_private_root /data/adb/hybrid-mount; then
  echo 'symlinked private root was accepted' >&2
  exit 1
fi
[ -d "$link_root_target" ]

echo 'Private data safety tests passed'
