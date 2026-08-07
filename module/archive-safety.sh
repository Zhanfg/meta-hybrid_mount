# Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
#
# SPDX-License-Identifier: GPL-3.0-only

REHYBIRD_ARCHIVE_MAX_FILE_BYTES=33554432
REHYBIRD_ARCHIVE_MAX_TOTAL_BYTES=100663296
REHYBIRD_ARCHIVE_MAX_ENTRIES=5000

rehybird_archive_error() {
  printf 'REHYBIRD archive safety error: %s\n' "$*" >&2
  return 1
}

rehybird_build_archive_manifest() {
  archive="$1"
  manifest="$2"
  listing="$manifest.listing"
  rm -f "$manifest" "$listing"

  unzip -l "$archive" >"$listing" 2>/dev/null || {
    rm -f "$listing"
    rehybird_archive_error 'cannot list archive'
    return 1
  }

  awk \
    -v max_file="$REHYBIRD_ARCHIVE_MAX_FILE_BYTES" \
    -v max_total="$REHYBIRD_ARCHIVE_MAX_TOTAL_BYTES" \
    -v max_entries="$REHYBIRD_ARCHIVE_MAX_ENTRIES" '
    $1 ~ /^[0-9]+$/ && $2 ~ /^[0-9-]+$/ && $3 ~ /^[0-9:]+$/ {
      size = $1 + 0
      name = $0
      sub(/^[[:space:]]*[0-9]+[[:space:]]+[0-9-]+[[:space:]]+[0-9:]+[[:space:]]+/, "", name)
      if (name == "") next

      count++
      total += size
      if (size > max_file || total > max_total || count > max_entries) bad = 1
      if (seen[name]++) bad = 1
      if (name !~ /^[A-Za-z0-9._\/-]+$/) bad = 1
      if (name ~ /^\// || name ~ /^[A-Za-z]:/ || name ~ /(^|\/)\.\.(\/|$)/ || name ~ /(^|\/)\.(\/|$)/ || name ~ /\/\// || name ~ /\\/) bad = 1
      printf "%d\t%s\n", size, name
    }
    END {
      if (count == 0 || bad) exit 1
    }
  ' "$listing" >"$manifest"
  status=$?
  rm -f "$listing"
  if [ "$status" -ne 0 ]; then
    rm -f "$manifest"
    rehybird_archive_error 'archive layout, size, name, or duplicate-entry validation failed'
    return 1
  fi
  chmod 600 "$manifest" 2>/dev/null || true
}

rehybird_extract_regular_archive() {
  archive="$1"
  destination="$2"
  manifest="$3"

  [ -f "$manifest" ] || {
    rehybird_archive_error 'validated manifest is missing'
    return 1
  }
  [ ! -e "$destination" ] && [ ! -L "$destination" ] || {
    rehybird_archive_error "destination already exists: $destination"
    return 1
  }
  mkdir -p "$destination" || return 1
  chmod 700 "$destination" 2>/dev/null || true

  tab="$(printf '\t')"
  while IFS="$tab" read -r expected_size entry; do
    [ -n "$entry" ] || continue
    case "$entry" in
    */)
      mkdir -p "$destination/${entry%/}" || return 1
      ;;
    *)
      parent="${entry%/*}"
      if [ "$parent" != "$entry" ]; then
        mkdir -p "$destination/$parent" || return 1
      fi
      output="$destination/$entry"
      [ ! -d "$output" ] || {
        rehybird_archive_error "file conflicts with directory: $entry"
        return 1
      }
      if ! unzip -p "$archive" "$entry" >"$output"; then
        rm -f "$output"
        rehybird_archive_error "failed to extract regular entry: $entry"
        return 1
      fi
      actual_size="$(wc -c <"$output" | tr -d '[:space:]')"
      if [ "$actual_size" != "$expected_size" ]; then
        rehybird_archive_error "size mismatch for $entry: expected=$expected_size actual=$actual_size"
        return 1
      fi
      ;;
    esac
  done <"$manifest"
}

rehybird_replace_module_tree() {
  staged_tree="$1"
  module_path="$2"
  backup_path="${module_path}.rehybird-extract-backup.$$"
  had_previous=false

  [ -d "$staged_tree" ] || return 1
  case "$module_path" in
  '' | / | /data | /data/adb | /data/adb/modules | /data/adb/modules_update)
    rehybird_archive_error "refusing unsafe module path replacement: $module_path"
    return 1
    ;;
  esac
  [ ! -e "$backup_path" ] && [ ! -L "$backup_path" ] || {
    rehybird_archive_error "stale extraction backup already exists: $backup_path"
    return 1
  }

  if [ -e "$module_path" ] || [ -L "$module_path" ]; then
    mv "$module_path" "$backup_path" || return 1
    had_previous=true
  fi

  if ! mkdir -p "$module_path" || ! cp -af "$staged_tree/." "$module_path/"; then
    rm -rf "$module_path" || true
    if [ "$had_previous" = true ]; then
      mv "$backup_path" "$module_path" || {
        rehybird_archive_error 'module tree replacement and rollback both failed'
        return 3
      }
    fi
    return 1
  fi

  if [ "$had_previous" = true ] && ! rm -rf "$backup_path"; then
    rm -rf "$module_path" || true
    if mv "$backup_path" "$module_path"; then
      rehybird_archive_error 'backup cleanup failed; previous module tree was restored'
      return 2
    fi
    rehybird_archive_error 'backup cleanup and rollback both failed'
    return 3
  fi

  return 0
}
