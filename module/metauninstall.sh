# Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
#
# SPDX-License-Identifier: GPL-3.0-only

MNT_DIR="/data/adb/hybrid-mount/mnt"
SCRIPT_DIR="${0%/*}"
SAFETY_LIB="$SCRIPT_DIR/metasafety.sh"

if [ "$SCRIPT_DIR" = "$0" ] || [ ! -r "$SAFETY_LIB" ]; then
  SAFETY_LIB="/data/adb/modules/hybrid_mount/metasafety.sh"
fi
if [ ! -r "$SAFETY_LIB" ]; then
  echo "REHYBIRD: safety helper unavailable; refusing metamodule cleanup" >&2
  exit 0
fi
# shellcheck source=module/metasafety.sh
. "$SAFETY_LIB"

if ! rehybird_valid_module_id "${MODULE_ID:-}"; then
  echo "REHYBIRD: invalid MODULE_ID; refusing metamodule cleanup" >&2
  exit 0
fi
if ! mountpoint -q "$MNT_DIR" 2>/dev/null; then
  exit 0
fi

MOD_IMG_DIR="$MNT_DIR/$MODULE_ID"
if [ -d "$MOD_IMG_DIR" ]; then
  rehybird_remove_path "$MOD_IMG_DIR" || {
    echo "REHYBIRD: failed to remove module image directory: $MOD_IMG_DIR" >&2
    exit 1
  }
fi
exit 0
