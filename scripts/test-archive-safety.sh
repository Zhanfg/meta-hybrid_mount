#!/usr/bin/env bash
# Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
#
# SPDX-License-Identifier: GPL-3.0-only

set -euo pipefail

# shellcheck source=module/archive-safety.sh
. module/archive-safety.sh

temp_dir="$(mktemp -d)"
trap 'rm -rf "$temp_dir"' EXIT

python3 - "$temp_dir" <<'PY'
import stat
import sys
import warnings
import zipfile
from pathlib import Path

root = Path(sys.argv[1])

with zipfile.ZipFile(root / "valid.zip", "w", zipfile.ZIP_DEFLATED) as archive:
    archive.writestr("directory/", b"")
    archive.writestr("directory/file.txt", b"hello")
    archive.writestr("root.txt", b"world")

with zipfile.ZipFile(root / "traversal.zip", "w") as archive:
    archive.writestr("../outside.txt", b"escape")

with warnings.catch_warnings():
    warnings.simplefilter("ignore", UserWarning)
    with zipfile.ZipFile(root / "duplicate.zip", "w") as archive:
        archive.writestr("duplicate.txt", b"one")
        archive.writestr("duplicate.txt", b"two")

with zipfile.ZipFile(root / "bad-name.zip", "w") as archive:
    archive.writestr("file with spaces.txt", b"bad")

with zipfile.ZipFile(root / "symlink-chain.zip", "w") as archive:
    link = zipfile.ZipInfo("link")
    link.create_system = 3
    link.external_attr = (stat.S_IFLNK | 0o777) << 16
    archive.writestr(link, b"../outside")
    archive.writestr("link/payload.txt", b"must-not-escape")

with zipfile.ZipFile(root / "reverse-conflict.zip", "w") as archive:
    archive.writestr("node/child.txt", b"child")
    archive.writestr("node", b"file")

with zipfile.ZipFile(root / "oversized.zip", "w") as archive:
    archive.writestr("large.bin", b"12345")
PY

expect_manifest_failure() {
  archive="$1"
  label="$2"
  manifest="$temp_dir/${label}.manifest"
  if rehybird_build_archive_manifest "$archive" "$manifest" >/dev/null 2>&1; then
    echo "unsafe archive manifest accepted: $label" >&2
    exit 1
  fi
  [ ! -e "$manifest" ]
}

valid_manifest="$temp_dir/valid.manifest"
valid_tree="$temp_dir/valid-tree"
rehybird_build_archive_manifest "$temp_dir/valid.zip" "$valid_manifest"
rehybird_extract_regular_archive "$temp_dir/valid.zip" "$valid_tree" "$valid_manifest"
[ -f "$valid_tree/directory/file.txt" ]
[ "$(cat "$valid_tree/directory/file.txt")" = hello ]
[ "$(cat "$valid_tree/root.txt")" = world ]
[ -z "$(find "$valid_tree" -type l -print -quit)" ]

expect_manifest_failure "$temp_dir/traversal.zip" traversal
expect_manifest_failure "$temp_dir/duplicate.zip" duplicate
expect_manifest_failure "$temp_dir/bad-name.zip" bad-name

symlink_manifest="$temp_dir/symlink-chain.manifest"
symlink_tree="$temp_dir/symlink-tree"
outside_file="$temp_dir/outside/payload.txt"
rehybird_build_archive_manifest "$temp_dir/symlink-chain.zip" "$symlink_manifest"
set +e
rehybird_extract_regular_archive \
  "$temp_dir/symlink-chain.zip" \
  "$symlink_tree" \
  "$symlink_manifest" >/dev/null 2>&1
symlink_status=$?
set -e
[ "$symlink_status" -ne 0 ]
[ ! -e "$outside_file" ]
[ ! -L "$symlink_tree/link" ]
[ -f "$symlink_tree/link" ]

conflict_manifest="$temp_dir/reverse-conflict.manifest"
conflict_tree="$temp_dir/conflict-tree"
rehybird_build_archive_manifest "$temp_dir/reverse-conflict.zip" "$conflict_manifest"
set +e
rehybird_extract_regular_archive \
  "$temp_dir/reverse-conflict.zip" \
  "$conflict_tree" \
  "$conflict_manifest" >/dev/null 2>&1
conflict_status=$?
set -e
[ "$conflict_status" -ne 0 ]

original_max_file="$REHYBIRD_ARCHIVE_MAX_FILE_BYTES"
REHYBIRD_ARCHIVE_MAX_FILE_BYTES=4
expect_manifest_failure "$temp_dir/oversized.zip" oversized
REHYBIRD_ARCHIVE_MAX_FILE_BYTES="$original_max_file"

existing_destination="$temp_dir/existing-destination"
mkdir -p "$existing_destination"
if rehybird_extract_regular_archive \
  "$temp_dir/valid.zip" \
  "$existing_destination" \
  "$valid_manifest" >/dev/null 2>&1; then
  echo 'existing extraction destination was accepted' >&2
  exit 1
fi

symlink_destination="$temp_dir/symlink-destination"
ln -s "$temp_dir/valid-tree" "$symlink_destination"
if rehybird_extract_regular_archive \
  "$temp_dir/valid.zip" \
  "$symlink_destination" \
  "$valid_manifest" >/dev/null 2>&1; then
  echo 'symlink extraction destination was accepted' >&2
  exit 1
fi

active="$temp_dir/modules/alpha"
staged="$temp_dir/staged-alpha"
mkdir -p "$active" "$staged"
printf 'old\n' >"$active/version"
printf 'new\n' >"$staged/version"
rehybird_replace_module_tree "$staged" "$active"
[ "$(cat "$active/version")" = new ]
[ -z "$(find "$temp_dir/modules" -maxdepth 1 -name 'alpha.rehybird-extract-backup.*' -print -quit)" ]

rollback_active="$temp_dir/modules/rollback"
rollback_staged="$temp_dir/staged-rollback"
mkdir -p "$rollback_active" "$rollback_staged" "$temp_dir/fail-cp-bin"
printf 'old\n' >"$rollback_active/version"
printf 'new\n' >"$rollback_staged/version"
cat >"$temp_dir/fail-cp-bin/cp" <<'EOF'
#!/bin/sh
exit 1
EOF
chmod 755 "$temp_dir/fail-cp-bin/cp"
old_path="$PATH"
PATH="$temp_dir/fail-cp-bin:$PATH"
set +e
rehybird_replace_module_tree "$rollback_staged" "$rollback_active" >/dev/null 2>&1
rollback_status=$?
set -e
PATH="$old_path"
[ "$rollback_status" = 1 ]
[ "$(cat "$rollback_active/version")" = old ]
[ -z "$(find "$temp_dir/modules" -maxdepth 1 -name 'rollback.rehybird-extract-backup.*' -print -quit)" ]

cleanup_active="$temp_dir/modules/cleanup"
cleanup_staged="$temp_dir/staged-cleanup"
mkdir -p "$cleanup_active" "$cleanup_staged" "$temp_dir/fail-backup-rm-bin"
printf 'old\n' >"$cleanup_active/version"
printf 'new\n' >"$cleanup_staged/version"
real_rm="$(command -v rm)"
cat >"$temp_dir/fail-backup-rm-bin/rm" <<EOF
#!/bin/sh
case "\$*" in
  *rehybird-extract-backup*) exit 1 ;;
esac
exec "$real_rm" "\$@"
EOF
chmod 755 "$temp_dir/fail-backup-rm-bin/rm"
PATH="$temp_dir/fail-backup-rm-bin:$PATH"
set +e
rehybird_replace_module_tree "$cleanup_staged" "$cleanup_active" >/dev/null 2>&1
cleanup_status=$?
set -e
PATH="$old_path"
[ "$cleanup_status" = 2 ]
[ "$(cat "$cleanup_active/version")" = old ]
[ -z "$(find "$temp_dir/modules" -maxdepth 1 -name 'cleanup.rehybird-extract-backup.*' -print -quit)" ]

echo 'Archive safety tests passed'
