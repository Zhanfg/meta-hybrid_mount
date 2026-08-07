# Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
#
# SPDX-License-Identifier: GPL-3.0-only

REHYBIRD_EXPECTED_LKMS="
android12-5.10_arm64_kasumi_lkm.ko
android13-5.10_arm64_kasumi_lkm.ko
android13-5.15_arm64_kasumi_lkm.ko
android14-5.15_arm64_kasumi_lkm.ko
android14-6.1_arm64_kasumi_lkm.ko
android15-6.6_arm64_kasumi_lkm.ko
android16-6.12_arm64_kasumi_lkm.ko
"

rehybird_integrity_error() {
  printf 'REHYBIRD package integrity error: %s\n' "$*" >&2
  return 1
}

rehybird_require_file() {
  root="$1"
  relative="$2"
  [ -f "$root/$relative" ] && [ -s "$root/$relative" ] \
    || rehybird_integrity_error "missing, empty, or non-regular file: $relative"
}

rehybird_require_directory() {
  root="$1"
  relative="$2"
  [ -d "$root/$relative" ] || rehybird_integrity_error "missing directory: $relative"
}

rehybird_single_property() {
  property_file="$1"
  property_name="$2"
  count="$(grep -c "^${property_name}=" "$property_file" 2>/dev/null || true)"
  [ "$count" = 1 ] || {
    rehybird_integrity_error "module.prop must contain exactly one ${property_name}= entry"
    return 1
  }
  sed -n "s/^${property_name}=//p" "$property_file"
}

rehybird_validate_no_links_or_special_files() {
  root="$1"
  if find "$root" -type l -print 2>/dev/null | grep -q .; then
    rehybird_integrity_error 'package contains a symbolic link'
    return 1
  fi
  if find "$root" \( -type b -o -type c -o -type p -o -type s \) -print 2>/dev/null | grep -q .; then
    rehybird_integrity_error 'package contains a device, FIFO, or socket'
    return 1
  fi
}

rehybird_validate_top_level() {
  root="$1"
  for candidate in "$root"/* "$root"/.[!.]* "$root"/..?*; do
    [ -e "$candidate" ] || [ -L "$candidate" ] || continue
    name="${candidate##*/}"
    case "$name" in
    .nano | archive-safety.sh | binaries | config.toml | customize.sh | kasumi_lkm | launcher.png | metainstall.sh | metamount.sh | metasafety.sh | metauninstall.sh | module.prop | module_blacklist.toml | package-integrity.sh | sepolicy.rule | uninstall-safety.sh | uninstall.sh | webroot)
      ;;
    *)
      rehybird_integrity_error "unexpected top-level package entry: $name"
      return 1
      ;;
    esac
  done
}

rehybird_validate_binary_directory() {
  root="$1"
  rehybird_require_directory "$root" binaries || return 1
  count=0
  for candidate in "$root/binaries"/* "$root/binaries"/.[!.]* "$root/binaries"/..?*; do
    [ -e "$candidate" ] || [ -L "$candidate" ] || continue
    count=$((count + 1))
    [ "${candidate##*/}" = hybrid-mount ] || {
      rehybird_integrity_error "unexpected binary payload: ${candidate##*/}"
      return 1
    }
  done
  [ "$count" = 1 ] || {
    rehybird_integrity_error "unexpected binary payload count: $count"
    return 1
  }
}

rehybird_validate_shell_scripts() {
  root="$1"
  for script in \
    archive-safety.sh \
    customize.sh \
    metainstall.sh \
    metamount.sh \
    metasafety.sh \
    metauninstall.sh \
    package-integrity.sh \
    uninstall-safety.sh \
    uninstall.sh; do
    sh -n "$root/$script" || {
      rehybird_integrity_error "shell syntax check failed: $script"
      return 1
    }
  done
}

rehybird_validate_full_lkms() {
  root="$1"
  lkm_dir="$root/kasumi_lkm"
  rehybird_require_directory "$root" kasumi_lkm || return 1

  expected_count=0
  for expected in $REHYBIRD_EXPECTED_LKMS; do
    expected_count=$((expected_count + 1))
    [ -f "$lkm_dir/$expected" ] && [ -s "$lkm_dir/$expected" ] || {
      rehybird_integrity_error "missing, empty, or non-regular Kasumi LKM: $expected"
      return 1
    }
  done

  actual_count=0
  for candidate in "$lkm_dir"/* "$lkm_dir"/.[!.]* "$lkm_dir"/..?*; do
    [ -e "$candidate" ] || [ -L "$candidate" ] || continue
    actual_count=$((actual_count + 1))
    candidate_name="${candidate##*/}"
    known=false
    for expected in $REHYBIRD_EXPECTED_LKMS; do
      if [ "$candidate_name" = "$expected" ]; then
        known=true
        break
      fi
    done
    if [ "$known" != true ]; then
      rehybird_integrity_error "unexpected Kasumi payload: $candidate_name"
      return 1
    fi
  done

  [ "$actual_count" = "$expected_count" ] || {
    rehybird_integrity_error "unexpected Kasumi LKM count: $actual_count (expected $expected_count)"
    return 1
  }
}

rehybird_validate_package_tree() {
  root="$1"
  [ -d "$root" ] || {
    rehybird_integrity_error "package root is unavailable: $root"
    return 1
  }

  rehybird_validate_no_links_or_special_files "$root" || return 1
  rehybird_validate_top_level "$root" || return 1

  for required in \
    module.prop \
    archive-safety.sh \
    config.toml \
    module_blacklist.toml \
    customize.sh \
    metainstall.sh \
    metamount.sh \
    metasafety.sh \
    metauninstall.sh \
    package-integrity.sh \
    sepolicy.rule \
    uninstall-safety.sh \
    uninstall.sh \
    binaries/hybrid-mount; do
    rehybird_require_file "$root" "$required" || return 1
  done
  rehybird_validate_binary_directory "$root" || return 1

  module_id="$(rehybird_single_property "$root/module.prop" id)" || return 1
  [ "$module_id" = hybrid_mount ] || {
    rehybird_integrity_error "unexpected module id: $module_id"
    return 1
  }

  module_name="$(rehybird_single_property "$root/module.prop" name)" || return 1
  case "$module_name" in
  'Hybrid Mount Fork')
    REHYBIRD_PACKAGE_FLAVOR=full
    ;;
  'Hybrid Mount Fork Lite')
    REHYBIRD_PACKAGE_FLAVOR=lite
    ;;
  'Hybrid Mount Fork Nano')
    REHYBIRD_PACKAGE_FLAVOR=nano
    ;;
  *)
    rehybird_integrity_error "unknown package flavor name: $module_name"
    return 1
    ;;
  esac

  rehybird_validate_shell_scripts "$root" || return 1

  case "$REHYBIRD_PACKAGE_FLAVOR" in
  full)
    rehybird_require_directory "$root" webroot || return 1
    rehybird_require_file "$root" webroot/index.html || return 1
    rehybird_require_file "$root" launcher.png || return 1
    [ ! -e "$root/.nano" ] || {
      rehybird_integrity_error 'Full package contains Nano marker'
      return 1
    }
    rehybird_validate_full_lkms "$root" || return 1
    ;;
  lite)
    rehybird_require_directory "$root" webroot || return 1
    rehybird_require_file "$root" webroot/index.html || return 1
    rehybird_require_file "$root" launcher.png || return 1
    [ ! -e "$root/.nano" ] || {
      rehybird_integrity_error 'Lite package contains Nano marker'
      return 1
    }
    [ ! -e "$root/kasumi_lkm" ] || {
      rehybird_integrity_error 'Lite package unexpectedly contains Kasumi assets'
      return 1
    }
    ;;
  nano)
    [ -f "$root/.nano" ] || {
      rehybird_integrity_error 'Nano package marker is missing'
      return 1
    }
    for forbidden in webroot launcher.png kasumi_lkm; do
      [ ! -e "$root/$forbidden" ] || {
        rehybird_integrity_error "Nano package unexpectedly contains: $forbidden"
        return 1
      }
    done
    ;;
  esac

  export REHYBIRD_PACKAGE_FLAVOR
  return 0
}
