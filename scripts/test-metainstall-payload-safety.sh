#!/usr/bin/env bash
# Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
#
# SPDX-License-Identifier: GPL-3.0-only

set -euo pipefail

temp_dir="$(mktemp -d)"
trap 'rm -rf "$temp_dir"' EXIT

MANAGED_PARTITIONS="$(sed -n 's/^MANAGED_PARTITIONS="\(.*\)"$/\1/p' module/metainstall.sh | head -n 1)"
[ -n "$MANAGED_PARTITIONS" ]

eval "$(sed -n '/^module_has_managed_partitions() {/,/^}/p' module/metainstall.sh)"

default_root="$temp_dir/module"

expect_payload() {
  label="$1"
  shift
  MODPATH="$default_root-$label"
  mkdir -p "$MODPATH"
  "$@"
  if ! module_has_managed_partitions; then
    echo "mount payload was misclassified as config-only: $label" >&2
    exit 1
  fi
  rm -rf "$MODPATH"
}

expect_no_payload() {
  MODPATH="$default_root-config-only"
  mkdir -p "$MODPATH"
  printf 'key=value\n' >"$MODPATH/settings.conf"
  if module_has_managed_partitions; then
    echo 'config-only module was misclassified as a mount payload' >&2
    exit 1
  fi
}

make_system_app() {
  mkdir -p "$MODPATH/system/app/Example"
  printf apk >"$MODPATH/system/app/Example/base.apk"
}

make_system_etc() {
  mkdir -p "$MODPATH/system/etc"
  printf config >"$MODPATH/system/etc/example.conf"
}

make_vendor_payload() {
  mkdir -p "$MODPATH/vendor/etc"
  printf config >"$MODPATH/vendor/etc/example.conf"
}

make_broken_partition_link() {
  ln -s /does/not/exist "$MODPATH/vendor"
}

make_system_file() {
  printf not-a-directory >"$MODPATH/system"
}

expect_payload system-app make_system_app
expect_payload system-etc make_system_etc
expect_payload vendor make_vendor_payload
expect_payload broken-vendor-link make_broken_partition_link
expect_payload system-file make_system_file
expect_no_payload

echo 'Metainstall payload classification tests passed'
