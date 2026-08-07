#!/usr/bin/env bash
# Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
#
# SPDX-License-Identifier: GPL-3.0-only

set -euo pipefail

# shellcheck source=module/package-integrity.sh
. module/package-integrity.sh

temp_dir="$(mktemp -d)"
trap 'rm -rf "$temp_dir"' EXIT

create_common_tree() {
  root="$1"
  module_name="$2"
  mkdir -p "$root/binaries"
  printf 'id=hybrid_mount\nname=%s\nversion=4.2.0-9999\nversionCode=1999999999\nmetamodule=1\n' \
    "$module_name" >"$root/module.prop"
  cp module/config.toml "$root/config.toml"
  cp module/module_blacklist.toml "$root/module_blacklist.toml"
  cp module/customize.sh "$root/customize.sh"
  cp module/metainstall.sh "$root/metainstall.sh"
  cp module/metamount.sh "$root/metamount.sh"
  cp module/metasafety.sh "$root/metasafety.sh"
  cp module/metauninstall.sh "$root/metauninstall.sh"
  cp module/package-integrity.sh "$root/package-integrity.sh"
  cp module/sepolicy.rule "$root/sepolicy.rule"
  cp module/uninstall-safety.sh "$root/uninstall-safety.sh"
  cp module/uninstall.sh "$root/uninstall.sh"
  printf '\177ELFmock\n' >"$root/binaries/hybrid-mount"
}

create_webui() {
  root="$1"
  mkdir -p "$root/webroot"
  printf '<!doctype html>\n' >"$root/webroot/index.html"
  printf 'png\n' >"$root/launcher.png"
}

create_full_lkms() {
  root="$1"
  mkdir -p "$root/kasumi_lkm"
  for lkm in $REHYBIRD_EXPECTED_LKMS; do
    printf 'ko:%s\n' "$lkm" >"$root/kasumi_lkm/$lkm"
  done
}

expect_valid() {
  root="$1"
  expected_flavor="$2"
  rehybird_validate_package_tree "$root"
  [ "$REHYBIRD_PACKAGE_FLAVOR" = "$expected_flavor" ]
}

expect_invalid() {
  root="$1"
  label="$2"
  if rehybird_validate_package_tree "$root" >/dev/null 2>&1; then
    echo "invalid package accepted: $label" >&2
    exit 1
  fi
}

full="$temp_dir/full"
create_common_tree "$full" 'Hybrid Mount Fork'
create_webui "$full"
create_full_lkms "$full"
expect_valid "$full" full

lite="$temp_dir/lite"
create_common_tree "$lite" 'Hybrid Mount Fork Lite'
create_webui "$lite"
expect_valid "$lite" lite

nano="$temp_dir/nano"
create_common_tree "$nano" 'Hybrid Mount Fork Nano'
printf 'nano\n' >"$nano/.nano"
expect_valid "$nano" nano

missing_lkm="$temp_dir/missing-lkm"
cp -a "$full" "$missing_lkm"
rm "$missing_lkm/kasumi_lkm/android15-6.6_arm64_kasumi_lkm.ko"
expect_invalid "$missing_lkm" 'Full missing target-device KMI'

extra_lkm="$temp_dir/extra-lkm"
cp -a "$full" "$extra_lkm"
printf 'unexpected\n' >"$extra_lkm/kasumi_lkm/android15-6.6_arm64_unknown.ko"
expect_invalid "$extra_lkm" 'Full with unexpected LKM'

mixed_lite="$temp_dir/mixed-lite"
cp -a "$lite" "$mixed_lite"
create_full_lkms "$mixed_lite"
expect_invalid "$mixed_lite" 'Lite containing Kasumi assets'

mixed_nano="$temp_dir/mixed-nano"
cp -a "$nano" "$mixed_nano"
create_webui "$mixed_nano"
expect_invalid "$mixed_nano" 'Nano containing WebUI assets'

missing_marker="$temp_dir/missing-marker"
cp -a "$nano" "$missing_marker"
rm "$missing_marker/.nano"
expect_invalid "$missing_marker" 'Nano without marker'

duplicate_name="$temp_dir/duplicate-name"
cp -a "$lite" "$duplicate_name"
printf 'name=Hybrid Mount Fork\n' >>"$duplicate_name/module.prop"
expect_invalid "$duplicate_name" 'duplicate flavor name'

wrong_id="$temp_dir/wrong-id"
cp -a "$lite" "$wrong_id"
sed -i 's/^id=.*/id=other_module/' "$wrong_id/module.prop"
expect_invalid "$wrong_id" 'wrong module ID'

missing_validator="$temp_dir/missing-validator"
cp -a "$lite" "$missing_validator"
rm "$missing_validator/package-integrity.sh"
expect_invalid "$missing_validator" 'missing installed validator'

missing_uninstall_guard="$temp_dir/missing-uninstall-guard"
cp -a "$lite" "$missing_uninstall_guard"
rm "$missing_uninstall_guard/uninstall-safety.sh"
expect_invalid "$missing_uninstall_guard" 'missing uninstall safety helper'

bad_shell="$temp_dir/bad-shell"
cp -a "$lite" "$bad_shell"
printf 'if then\n' >>"$bad_shell/metamount.sh"
expect_invalid "$bad_shell" 'invalid shell syntax'

echo 'Package integrity tests passed'
