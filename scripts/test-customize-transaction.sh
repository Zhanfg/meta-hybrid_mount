#!/usr/bin/env bash
# Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
#
# SPDX-License-Identifier: GPL-3.0-only

set -euo pipefail

repo_root="$(pwd)"
temp_dir="$(mktemp -d)"
module_test_root="/data/adb/modules/rehybird_ci_transaction"
private_root="/data/adb/hybrid-mount"
module_backup=""
private_backup=""

cleanup() {
  rm -rf "$module_test_root"
  rm -rf "$private_root"
  if [ -n "$module_backup" ] && [ -e "$module_backup" ]; then
    mv "$module_backup" "$module_test_root"
  fi
  if [ -n "$private_backup" ] && [ -e "$private_backup" ]; then
    mv "$private_backup" "$private_root"
  fi
  rm -rf "$temp_dir"
}
trap cleanup EXIT

mkdir -p /data/adb/modules /data/local/tmp
if [ -e "$module_test_root" ] || [ -L "$module_test_root" ]; then
  module_backup="$temp_dir/original-module"
  mv "$module_test_root" "$module_backup"
fi
if [ -e "$private_root" ] || [ -L "$private_root" ]; then
  private_backup="$temp_dir/original-private"
  mv "$private_root" "$private_backup"
fi

package_root="$temp_dir/package"
mkdir -p "$package_root/binaries"
for file in \
  archive-safety.sh \
  config.toml \
  customize.sh \
  metainstall.sh \
  metamount.sh \
  metasafety.sh \
  metauninstall.sh \
  module_blacklist.toml \
  package-integrity.sh \
  sepolicy.rule \
  uninstall-safety.sh \
  uninstall.sh; do
  cp "$repo_root/module/$file" "$package_root/$file"
done
printf 'mock-aarch64-binary\n' >"$package_root/binaries/hybrid-mount"
printf 'nano\n' >"$package_root/.nano"
cat >"$package_root/module.prop" <<'EOF'
id=hybrid_mount
name=Hybrid Mount Fork Nano
version=4.2.0-9999
versionCode=1999999999
metamodule=1
EOF

zip_path="$temp_dir/rehybird-nano.zip"
(
  cd "$package_root"
  zip -qr "$zip_path" .
)
unzip -tqq "$zip_path"

run_installer() {
  arch="$1"
  fail_recursive="$2"
  (
    set -u
    export APATCH=1
    export KSU=""
    export KSU_LATE_LOAD=""
    export ARCH="$arch"
    export ZIPFILE="$zip_path"
    export MODPATH="$module_test_root"

    ui_print() {
      :
    }
    abort() {
      printf 'installer abort: %s\n' "$*" >&2
      exit 97
    }
    set_perm() {
      path="$1"
      uid="$2"
      gid="$3"
      mode="$4"
      chown "$uid:$gid" "$path" && chmod "$mode" "$path"
    }
    set_perm_recursive() {
      root="$1"
      uid="$2"
      gid="$3"
      dir_mode="$4"
      file_mode="$5"
      if [ "$fail_recursive" = true ]; then
        return 1
      fi
      chown -R "$uid:$gid" "$root" || return 1
      find "$root" -type d -exec chmod "$dir_mode" {} + || return 1
      find "$root" -type f -exec chmod "$file_mode" {} + || return 1
    }

    # shellcheck disable=SC1090
    . "$repo_root/module/customize.sh"
  )
}

reset_old_module() {
  rm -rf "$module_test_root"
  mkdir -p "$module_test_root"
  printf 'old-tree\n' >"$module_test_root/old-marker"
}

# Success path: final package becomes active and private files are secured.
reset_old_module
rm -rf "$private_root"
run_installer arm64 false
[ ! -e "$module_test_root/old-marker" ]
[ -f "$module_test_root/hybrid-mount" ]
[ ! -e "$module_test_root/binaries" ]
[ -f "$private_root/config.toml" ]
[ -f "$private_root/module_blacklist.toml" ]
[ "$(stat -c %a "$private_root")" = 700 ]
[ "$(stat -c %a "$private_root/config.toml")" = 600 ]
[ "$(stat -c %a "$private_root/module_blacklist.toml")" = 600 ]

# Late failure: restore the previous module tree and remove private files/root
# created by this attempted installation.
reset_old_module
rm -rf "$private_root"
set +e
run_installer arm64 true >/dev/null 2>&1
status=$?
set -e
[ "$status" -ne 0 ]
[ "$(cat "$module_test_root/old-marker")" = old-tree ]
[ ! -e "$private_root/config.toml" ]
[ ! -e "$private_root/module_blacklist.toml" ]
[ ! -e "$private_root" ]

# Existing private configuration is never deleted by a failed upgrade.
reset_old_module
mkdir -p "$private_root"
chmod 700 "$private_root"
printf 'preserved-config\n' >"$private_root/config.toml"
printf 'preserved-blacklist\n' >"$private_root/module_blacklist.toml"
chmod 600 "$private_root/config.toml" "$private_root/module_blacklist.toml"
set +e
run_installer arm64 true >/dev/null 2>&1
status=$?
set -e
[ "$status" -ne 0 ]
[ "$(cat "$module_test_root/old-marker")" = old-tree ]
[ "$(cat "$private_root/config.toml")" = preserved-config ]
[ "$(cat "$private_root/module_blacklist.toml")" = preserved-blacklist ]

# Architecture rejection must occur before the old module tree is moved.
reset_old_module
rm -rf "$private_root"
set +e
run_installer x86_64 false >/dev/null 2>&1
status=$?
set -e
[ "$status" -ne 0 ]
[ "$(cat "$module_test_root/old-marker")" = old-tree ]
[ ! -e "$private_root" ]

echo 'Customize transaction tests passed'
