# Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
#
# SPDX-License-Identifier: GPL-3.0-only

export KSU_HAS_METAMODULE="true"
export KSU_METAMODULE="hybrid-mount"
BASE_DIR="/data/adb/hybrid-mount"
MANAGED_PARTITIONS="system odm product system_ext vendor apex mi_ext my_bigball my_carrier my_company my_engineering my_heytap my_manifest my_preload my_product my_region my_reserve my_stock oem optics prism"
MODE_MARKERS="overlay magic"
SELF_MOUNTING_MODULE_BLOCKLIST="scene_swap_controller AAaTempSpoof"
NANO_MODE=false
META_SAFETY_AVAILABLE=false

load_meta_safety() {
  local script_dir candidate
  script_dir="${0%/*}"
  for candidate in \
    "$script_dir/metasafety.sh" \
    "/data/adb/modules/hybrid_mount/metasafety.sh" \
    "/data/adb/modules_update/hybrid_mount/metasafety.sh"; do
    if [ -r "$candidate" ]; then
      # shellcheck source=module/metasafety.sh
      . "$candidate"
      META_SAFETY_AVAILABLE=true
      return 0
    fi
  done
  return 1
}

load_meta_safety || true

detect_nano_mode() {
  local script_dir="${0%/*}"
  if [ "$script_dir" != "$0" ] && [ -f "$script_dir/.nano" ]; then
    return 0
  fi
  if [ -f "/data/adb/modules/hybrid_mount/.nano" ]; then
    return 0
  fi
  if [ -f "/data/adb/modules_update/hybrid_mount/.nano" ]; then
    return 0
  fi
  return 1
}

read_default_mount_mode() {
  if [ ! -f "$BASE_DIR/config.toml" ]; then
    return 1
  fi
  local default_mode
  default_mode=$(grep -E '^[[:space:]]*default_mode[[:space:]]*=' "$BASE_DIR/config.toml" | head -n 1 | sed 's/.*=[[:space:]]*"\([^"]*\)".*/\1/')
  case "$default_mode" in
  overlay | magic)
    printf '%s\n' "$default_mode"
    ;;
  *)
    return 1
    ;;
  esac
}

mode_label() {
  case "$1" in
  overlay)
    printf '%s\n' "OverlayFS"
    ;;
  magic)
    printf '%s\n' "Magic Mount"
    ;;
  *)
    printf '%s\n' "$1"
    ;;
  esac
}

wait_volume_key_or_timeout() {
  local timeout_seconds=$1
  local start_time=$(date +%s)
  while true; do
    local current_time=$(date +%s)
    if [ $((current_time - start_time)) -ge "$timeout_seconds" ]; then
      printf 'timeout\n'
      return 0
    fi
    local key_event
    key_event=$(timeout 0.5 getevent -l 2>/dev/null || true)
    if echo "$key_event" | grep -q "KEY_VOLUMEUP"; then
      printf 'up\n'
      return 0
    elif echo "$key_event" | grep -q "KEY_VOLUMEDOWN"; then
      printf 'down\n'
      return 0
    fi
    sleep 0.1
  done
}

module_has_managed_partitions() {
  for partition in $MANAGED_PARTITIONS; do
    if [ -d "$MODPATH/system/$partition" ] || [ -d "$MODPATH/$partition" ]; then
      return 0
    fi
  done
  return 1
}

normalize_symlinked_partition_layout() {
  local partition
  for partition in vendor product system_ext; do
    if [ -L "/system/$partition" ] && [ -d "$MODPATH/system/$partition" ]; then
      if [ -d "$MODPATH/$partition" ]; then
        cp -a "$MODPATH/system/$partition/." "$MODPATH/$partition/" && rm -rf "$MODPATH/system/$partition"
      else
        mv "$MODPATH/system/$partition" "$MODPATH/$partition"
      fi
      ui_print "- normalized /system/$partition layout"
    fi
  done
}

current_module_id() {
  if [ -n "$KSU_MODULE" ]; then
    printf '%s\n' "$KSU_MODULE"
    return 0
  fi
  if [ -n "$AP_MODULE" ]; then
    printf '%s\n' "$AP_MODULE"
    return 0
  fi
  if [ -n "$MODID" ]; then
    printf '%s\n' "$MODID"
    return 0
  fi
  if [ -f "$MODPATH/module.prop" ]; then
    grep -m 1 '^id=' "$MODPATH/module.prop" | sed 's/^id=//'
  fi
}

mark_self_mounting_blocklisted_module() {
  local current_module blocked_id
  current_module="$(current_module_id)"
  if [ -z "$current_module" ]; then
    return 0
  fi

  for blocked_id in $SELF_MOUNTING_MODULE_BLOCKLIST; do
    if [ "$current_module" = "$blocked_id" ]; then
      ui_print "**********************************************"
      ui_print "! Module '$current_module' already has self-mounting logic!"
      ui_print "! Marking skip mount"
      ui_print "**********************************************"
      : >"$MODPATH/skip_mount"
      return 0
    fi
  done
}

current_mount_mode_marker() {
  for marker in $MODE_MARKERS; do
    if [ -f "$MODPATH/$marker" ]; then
      printf '%s\n' "$marker"
      return 0
    fi
  done
  return 1
}

clear_mount_mode_markers() {
  for marker in $MODE_MARKERS; do
    rm -f "$MODPATH/$marker"
  done
}

write_mount_mode_marker() {
  local mode="$1"
  clear_mount_mode_markers
  : >"$MODPATH/$mode"
}

prompt_module_mount_mode() {
  local default_mode default_label chosen_mode existing_mode

  existing_mode="$(current_mount_mode_marker || true)"
  if [ -n "$existing_mode" ]; then
    ui_print "- Existing module mount mode marker: $(mode_label "$existing_mode")"
    write_mount_mode_marker "$existing_mode"
    return 0
  fi

  default_mode="$(read_default_mount_mode)" || abort "! Missing or invalid default_mode in $BASE_DIR/config.toml"
  default_label="$(mode_label "$default_mode")"
  ui_print " "
  ui_print "========================================"
  ui_print "      Select Module Mount Mode         "
  ui_print "========================================"
  ui_print "  Volume Up (+): OverlayFS"
  ui_print "  Volume Down (-): Magic Mount"
  ui_print " "
  ui_print "  Defaulting to ${default_label} in 10 seconds"
  ui_print "========================================"

  chosen_mode="$default_mode"
  case "$(wait_volume_key_or_timeout 10)" in
  up)
    chosen_mode="overlay"
    ui_print "- Key Detected: Selected OverlayFS"
    ;;
  down)
    chosen_mode="magic"
    ui_print "- Key Detected: Selected Magic Mount"
    ;;
  timeout)
    ui_print "- Timeout: Selected ${default_label}"
    ;;
  esac

  write_mount_mode_marker "$chosen_mode"
  ui_print "- Marker written: $chosen_mode"
}

handle_partition() {
  echo 0 >/dev/null
  true
}

hybrid_handle_partition() {
  partition="$1"

  if [ ! -d "$MODPATH/system/$partition" ]; then
    return
  fi

  if [ -d "/$partition" ] && [ -L "/system/$partition" ]; then
    ln -sf "./system/$partition" "$MODPATH/$partition"
    ui_print "- handled /$partition"
  fi
}

cleanup_empty_system_dir() {
  if [ -d "$MODPATH/system" ] && [ -z "$(ls -A "$MODPATH/system" 2>/dev/null)" ]; then
    rmdir "$MODPATH/system" 2>/dev/null
    ui_print "- Removed empty /system directory (Skip system mount)"
  fi
}

mark_replace() {
  replace_target="$1"
  mkdir -p "$replace_target"
  setfattr -n trusted.overlay.opaque -v y "$replace_target"
}

ui_print "- Using Hybrid Mount metainstall"

install_module
mark_self_mounting_blocklisted_module
normalize_symlinked_partition_layout

if detect_nano_mode; then
  NANO_MODE=true
  ui_print "- Flavor: Nano (config-only)"
fi

for partition in $MANAGED_PARTITIONS; do
  hybrid_handle_partition "$partition"
done

cleanup_empty_system_dir

if [ "$NANO_MODE" = "true" ] && [ ! -f "$MODPATH/skip_mount" ] && module_has_managed_partitions; then
  prompt_module_mount_mode
fi

ui_print "- Installation complete"

metamodule_hot_install() {
  local active_dir update_dir backup_dir stage_status rollback_status

  # Hot install is currently only supported on KernelSU.
  if [ "${KSU:-}" != true ]; then
    return
  fi
  if [ "$META_SAFETY_AVAILABLE" != true ]; then
    ui_print "! REHYBIRD safety helper unavailable; keeping update staged for reboot"
    return
  fi
  if ! rehybird_valid_module_id "${MODID:-}"; then
    ui_print "! Invalid module ID; refusing hot install and keeping update staged"
    return
  fi
  if module_has_managed_partitions; then
    ui_print "- Module changes managed partitions; hot install is blocked for safety"
    ui_print "- Update remains staged and will apply after reboot"
    return
  fi
  if [ -n "${MODULE_HOT_RUN_SCRIPT:-}" ]; then
    ui_print "- Requested hot-run script is not executed by REHYBIRD safety mode"
    ui_print "- Update remains staged and will apply after reboot"
    return
  fi

  active_dir="/data/adb/modules/$MODID"
  update_dir="/data/adb/modules_update/$MODID"
  backup_dir="${active_dir}.rehybird-backup.$$"

  if [ ! -d "$active_dir" ] || [ ! -d "$update_dir" ]; then
    return
  fi

  rehybird_stage_module_replace "$active_dir" "$update_dir" "$backup_dir"
  stage_status=$?
  if [ "$stage_status" -ne 0 ]; then
    case "$stage_status" in
    15)
      abort "! Hot-install rollback failed; active module directory requires manual recovery"
      ;;
    *)
      ui_print "! Hot install could not be staged safely (code $stage_status)"
      ui_print "- Existing module remains active; update remains for reboot when possible"
      return
      ;;
    esac
  fi

  if ! mkdir -p "$update_dir" || ! cat "$active_dir/module.prop" >"$update_dir/module.prop"; then
    rehybird_rollback_module_replace "$active_dir" "$update_dir" "$backup_dir"
    rollback_status=$?
    if [ "$rollback_status" -ne 0 ]; then
      abort "! Hot-install metadata and rollback both failed (code $rollback_status)"
    fi
    ui_print "! Could not create KernelSU update stub; restored previous module"
    return
  fi

  if ! rehybird_commit_module_replace "$backup_dir"; then
    ui_print "! Hot install succeeded but backup cleanup failed: $backup_dir"
  fi

  (
    sleep 3
    rm -rf "$active_dir/update"
    rm -rf "$update_dir"
  ) &

  ui_print "- Config-only module hot install completed transactionally"
  ui_print "- Refresh module page after installation"
}

if [ "${MODULE_HOT_INSTALL_REQUEST:-}" = true ]; then
  metamodule_hot_install
fi
