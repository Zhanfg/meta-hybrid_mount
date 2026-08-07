# Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
#
# SPDX-License-Identifier: GPL-3.0-only

rehybird_mountinfo_references_path() {
  base_dir="$1"
  shift

  for mountinfo in "$@"; do
    [ -r "$mountinfo" ] || continue
    if grep -F "$base_dir" "$mountinfo" >/dev/null 2>&1; then
      return 0
    fi
  done
  return 1
}

rehybird_remove_inactive_base() {
  base_dir="$1"
  shift

  [ -e "$base_dir" ] || [ -L "$base_dir" ] || return 0

  if rehybird_mountinfo_references_path "$base_dir" "$@"; then
    return 10
  fi

  if [ -L "$base_dir" ]; then
    rm -f "$base_dir"
  else
    rm -rf "$base_dir"
  fi
}
