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

rehybird_stat_value() {
  stat_format="$1"
  stat_path="$2"
  if command -v busybox >/dev/null 2>&1; then
    busybox stat -c "$stat_format" "$stat_path"
  else
    stat -c "$stat_format" "$stat_path"
  fi
}

rehybird_prepare_private_root() {
  private_root="$1"

  if [ -L "$private_root" ]; then
    return 30
  fi
  if [ -e "$private_root" ] && [ ! -d "$private_root" ]; then
    return 31
  fi
  if [ ! -d "$private_root" ]; then
    mkdir -m 0700 "$private_root" || return 32
  fi
  [ -d "$private_root" ] && [ ! -L "$private_root" ] || return 33

  owner="$(rehybird_stat_value %u "$private_root")" || return 34
  [ "$owner" = 0 ] || return 35
  chown 0:0 "$private_root" || return 36
  chmod 0700 "$private_root" || return 37
  [ "$(rehybird_stat_value %u "$private_root")" = 0 ] || return 38
  [ "$(rehybird_stat_value %a "$private_root")" = 700 ] || return 39
}

rehybird_secure_existing_private_file() {
  private_file="$1"

  [ -f "$private_file" ] && [ ! -L "$private_file" ] || return 40
  [ "$(rehybird_stat_value %u "$private_file")" = 0 ] || return 41
  [ "$(rehybird_stat_value %h "$private_file")" = 1 ] || return 42

  chown 0:0 "$private_file" || return 43
  chmod 0600 "$private_file" || return 44
  [ -f "$private_file" ] && [ ! -L "$private_file" ] || return 45
  [ "$(rehybird_stat_value %u "$private_file")" = 0 ] || return 46
  [ "$(rehybird_stat_value %h "$private_file")" = 1 ] || return 47
  [ "$(rehybird_stat_value %a "$private_file")" = 600 ] || return 48
}

rehybird_install_private_file() {
  source_file="$1"
  target_file="$2"
  target_parent="${target_file%/*}"
  target_name="${target_file##*/}"
  temp_file="$target_parent/.${target_name}.rehybird-new.$$"

  [ -f "$source_file" ] && [ ! -L "$source_file" ] || return 50
  [ -d "$target_parent" ] && [ ! -L "$target_parent" ] || return 51
  [ "$(rehybird_stat_value %u "$target_parent")" = 0 ] || return 52
  if [ -e "$target_file" ] || [ -L "$target_file" ]; then
    return 53
  fi
  if [ -e "$temp_file" ] || [ -L "$temp_file" ]; then
    return 54
  fi

  umask 077
  if ! cat "$source_file" >"$temp_file"; then
    rm -f "$temp_file"
    return 55
  fi
  if ! chown 0:0 "$temp_file" || ! chmod 0600 "$temp_file"; then
    rm -f "$temp_file"
    return 56
  fi
  if ! rehybird_secure_existing_private_file "$temp_file"; then
    rm -f "$temp_file"
    return 57
  fi
  if ! rehybird_move_path "$temp_file" "$target_file"; then
    rm -f "$temp_file"
    return 58
  fi
  rehybird_secure_existing_private_file "$target_file" || return 59
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
