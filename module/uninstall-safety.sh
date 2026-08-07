# Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
#
# SPDX-License-Identifier: GPL-3.0-only

rehybird_mountinfo_references_path() {
  base_dir="$1"
  shift

  for mountinfo in "$@"; do
    [ -r "$mountinfo" ] || continue
    if awk -v base="$base_dir" '
      {
        start = index($0, base)
        while (start > 0) {
          suffix = substr($0, start + length(base))
          if (suffix == "" || suffix ~ /^[\/ ,:]/) {
            found = 1
            exit
          }
          remainder = substr($0, start + 1)
          next_start = index(remainder, base)
          if (next_start == 0) break
          start += next_start
        }
      }
      END { exit found ? 0 : 1 }
    ' "$mountinfo"; then
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
