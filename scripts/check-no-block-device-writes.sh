#!/usr/bin/env bash
# Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
#
# SPDX-License-Identifier: GPL-3.0-only

set -euo pipefail

readonly SELF='scripts/check-no-block-device-writes.sh'
readonly PATTERN='(/dev/block(/|[[:space:]"])|/dev/disk/by-partlabel/|write_raw_image|flash_image|nandwrite|fastboot[[:space:]]+flash|blockdev[[:space:]]+--setrw|sg_write_buffer|mmc[[:space:]]+write|dd[^\n]*of=[^[:space:]]*/dev/)'

mapfile -d '' files < <(
  git ls-files -z -- 'module/*.sh' 'src/**/*.rs' 'scripts/*.sh' \
    | while IFS= read -r -d '' file; do
        [ "$file" = "$SELF" ] || printf '%s\0' "$file"
      done
)

if [ "${#files[@]}" -eq 0 ]; then
  echo 'No production files found for block-device safety scan' >&2
  exit 1
fi

violations=0
for file in "${files[@]}"; do
  if grep -nE "$PATTERN" "$file"; then
    echo "Forbidden direct block-device write primitive in $file" >&2
    violations=1
  fi
done

if [ "$violations" -ne 0 ]; then
  echo 'Direct partition writes are forbidden in REHYBIRD production paths.' >&2
  exit 1
fi

echo "Block-device safety scan passed: ${#files[@]} files checked"
