# Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
#
# SPDX-License-Identifier: GPL-3.0-only

rehybird_valid_module_id() {
  module_id="$1"
  case "$module_id" in
  '' | [!A-Za-z]* | *[!A-Za-z0-9._-]*)
    return 1
    ;;
  esac
  [ "${#module_id}" -ge 2 ]
}

rehybird_move_path() {
  if command -v busybox >/dev/null 2>&1; then
    busybox mv "$1" "$2"
  else
    mv "$1" "$2"
  fi
}

rehybird_remove_path() {
  if command -v busybox >/dev/null 2>&1; then
    busybox rm -rf "$1"
  else
    rm -rf "$1"
  fi
}

# Move an installed module aside, then atomically promote its staged update.
# The backup remains until the caller has created KernelSU's update stub and
# explicitly commits the transaction.
rehybird_stage_module_replace() {
  active_dir="$1"
  update_dir="$2"
  backup_dir="$3"

  [ -d "$active_dir" ] || return 10
  [ -d "$update_dir" ] || return 11
  [ ! -e "$backup_dir" ] || return 12

  rehybird_move_path "$active_dir" "$backup_dir" || return 13
  if rehybird_move_path "$update_dir" "$active_dir"; then
    return 0
  fi

  if ! rehybird_move_path "$backup_dir" "$active_dir"; then
    return 15
  fi
  return 14
}

rehybird_rollback_module_replace() {
  active_dir="$1"
  update_dir="$2"
  backup_dir="$3"

  [ -d "$backup_dir" ] || return 20
  rehybird_remove_path "$update_dir" || return 21
  if [ -d "$active_dir" ]; then
    rehybird_move_path "$active_dir" "$update_dir" || return 22
  fi
  rehybird_move_path "$backup_dir" "$active_dir" || return 23
}

rehybird_commit_module_replace() {
  backup_dir="$1"
  [ -d "$backup_dir" ] || return 0
  rehybird_remove_path "$backup_dir"
}
