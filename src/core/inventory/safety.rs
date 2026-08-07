// Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-only

use std::{
    collections::VecDeque,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};

use crate::{
    defs,
    domain::{ModuleRules, MountMode},
};

const MAX_SCANNED_ENTRIES: usize = 100_000;
const MAX_DIRECTORY_DEPTH: usize = 64;

const CRITICAL_TREE_ROOTS: &[&str] = &[
    "system/etc/firmware",
    "vendor/firmware",
    "vendor/firmware_mnt",
    "vendor/bt_firmware",
    "vendor/etc/firmware",
    "vendor/rfs",
    "vendor/etc/bluetooth",
    "vendor/etc/qcril_database",
    "vendor/etc/radio",
    "vendor/etc/modem",
    "vendor/etc/init",
    "vendor/etc/vintf",
    "vendor/etc/selinux",
    "odm/firmware",
    "odm/bt_firmware",
    "odm/etc/firmware",
    "odm/etc/bluetooth",
    "odm/etc/radio",
    "odm/etc/modem",
    "odm/etc/init",
    "odm/etc/vintf",
    "odm/etc/selinux",
    "my_carrier/firmware",
    "my_carrier/bt_firmware",
    "my_carrier/etc/firmware",
    "my_carrier/etc/radio",
    "my_carrier/etc/modem",
    "my_carrier/etc/init",
    "my_carrier/etc/vintf",
];

const CRITICAL_REPLACE_PARENTS: &[&str] = &[
    "system",
    "vendor",
    "odm",
    "product",
    "system_ext",
    "apex",
    "my_carrier",
    "vendor/etc",
    "vendor/bin",
    "vendor/lib",
    "vendor/lib64",
    "odm/etc",
    "odm/bin",
    "odm/lib",
    "odm/lib64",
    "my_carrier/etc",
    "my_carrier/bin",
    "my_carrier/lib",
    "my_carrier/lib64",
];

const SENSITIVE_AREAS: &[&str] = &[
    "vendor/bin",
    "vendor/lib",
    "vendor/lib64",
    "vendor/etc/permissions",
    "vendor/etc/sysconfig",
    "odm/bin",
    "odm/lib",
    "odm/lib64",
    "odm/etc/permissions",
    "odm/etc/sysconfig",
    "my_carrier/bin",
    "my_carrier/lib",
    "my_carrier/lib64",
    "my_carrier/etc/permissions",
    "my_carrier/etc/sysconfig",
];

const SENSITIVE_IDENTIFIERS: &[&str] = &[
    "bluetooth",
    "android.hardware.radio",
    "vendor.qti.hardware.radio",
    "vendor.oplus.hardware.radio",
    "telephony",
    "qcril",
    "libril",
    "/rild",
    "modem",
    "libbt",
    "bt_vendor",
    "bt_firmware",
    "wcnss",
    "mpss",
];

const CRITICAL_EXACT_FILES: &[&str] = &[
    "vendor/ueventd.rc",
    "vendor/build.prop",
    "vendor/default.prop",
    "odm/ueventd.rc",
    "odm/build.prop",
    "my_carrier/build.prop",
];

const CRITICAL_FILE_PREFIXES: &[&str] = &[
    "vendor/etc/fstab.",
    "odm/etc/fstab.",
    "my_carrier/etc/fstab.",
];

pub(super) fn first_blocked_critical_path(
    module_root: &Path,
    rules: &ModuleRules,
) -> Result<Option<PathBuf>> {
    let mut queue = VecDeque::from([(module_root.to_path_buf(), PathBuf::new(), 0usize)]);
    let mut scanned_entries = 0usize;

    while let Some((directory, relative_directory, depth)) = queue.pop_front() {
        if depth > MAX_DIRECTORY_DEPTH {
            bail!(
                "module tree exceeds critical-path scan depth at {}",
                relative_directory.display()
            );
        }

        let entries = fs::read_dir(&directory)
            .with_context(|| format!("failed to inspect module path {}", directory.display()))?;
        for entry in entries {
            scanned_entries += 1;
            if scanned_entries > MAX_SCANNED_ENTRIES {
                bail!("module tree exceeds critical-path scan entry limit ({MAX_SCANNED_ENTRIES})");
            }

            let entry = entry.with_context(|| {
                format!("failed to enumerate module path {}", directory.display())
            })?;
            let relative_path = relative_directory.join(entry.file_name());
            let normalized_path = normalize_partition_alias(&relative_path);
            let file_type = entry
                .file_type()
                .with_context(|| format!("failed to inspect {}", entry.path().display()))?;

            let critical_replace = is_critical_replace_marker(&normalized_path);
            let partition_root_non_dir = is_partition_root(&normalized_path) && !file_type.is_dir();
            let blocked = is_critical_payload_path(&normalized_path)
                || critical_replace
                || partition_root_non_dir;
            if blocked {
                if blocked_entry_is_covered_by_ignore(
                    rules,
                    &relative_path,
                    &normalized_path,
                    file_type.is_dir(),
                    critical_replace,
                    partition_root_non_dir,
                ) {
                    crate::scoped_log!(
                        warn,
                        "inventory:safety",
                        "critical subtree excluded by an effective Ignore rule: path={}",
                        relative_path.display()
                    );
                    continue;
                }
                return Ok(Some(relative_path));
            }

            if file_type.is_dir() {
                queue.push_back((entry.path(), relative_path, depth + 1));
            }
        }
    }

    Ok(None)
}

fn blocked_entry_is_covered_by_ignore(
    rules: &ModuleRules,
    relative_path: &Path,
    normalized_path: &Path,
    is_directory: bool,
    is_replace_marker: bool,
    is_partition_root_non_dir: bool,
) -> bool {
    if is_partition_root_non_dir {
        return false;
    }

    let relative_rule_path = if is_directory && !is_replace_marker {
        relative_path
    } else {
        let Some(parent) = relative_path.parent() else {
            return false;
        };
        parent
    };
    if relative_rule_path.as_os_str().is_empty() {
        return false;
    }

    if matches!(rules.effective_mode(relative_rule_path), MountMode::Ignore) {
        return true;
    }

    let normalized_rule_path = if is_directory && !is_replace_marker {
        normalized_path
    } else {
        let Some(parent) = normalized_path.parent() else {
            return false;
        };
        parent
    };
    normalized_rule_path != relative_rule_path
        && !normalized_rule_path.as_os_str().is_empty()
        && matches!(
            rules.effective_mode(normalized_rule_path),
            MountMode::Ignore
        )
}

fn normalize_partition_alias(path: &Path) -> PathBuf {
    let mut components = path.components();
    let Some(first) = components.next() else {
        return PathBuf::new();
    };
    let first = first.as_os_str();
    if first != "system" {
        return path.to_path_buf();
    }

    let Some(second) = components.next() else {
        return path.to_path_buf();
    };
    let second_name = second.as_os_str();
    if !matches!(
        second_name.to_str(),
        Some("vendor" | "odm" | "product" | "system_ext")
    ) {
        return path.to_path_buf();
    }

    let mut normalized = PathBuf::from(second_name);
    for component in components {
        normalized.push(component.as_os_str());
    }
    normalized
}

fn is_partition_root(path: &Path) -> bool {
    if path == Path::new("system") {
        return true;
    }
    defs::MANAGED_PARTITIONS
        .iter()
        .any(|partition| path == Path::new(partition))
}

fn is_critical_replace_marker(path: &Path) -> bool {
    if path.file_name().and_then(|name| name.to_str()) != Some(".replace") {
        return false;
    }
    let Some(parent) = path.parent() else {
        return false;
    };

    CRITICAL_REPLACE_PARENTS
        .iter()
        .map(Path::new)
        .any(|critical_parent| parent == critical_parent)
        || CRITICAL_TREE_ROOTS
            .iter()
            .map(Path::new)
            .any(|root| parent == root || parent.starts_with(root))
}

fn is_critical_payload_path(path: &Path) -> bool {
    if CRITICAL_TREE_ROOTS
        .iter()
        .map(Path::new)
        .any(|root| path == root || path.starts_with(root))
    {
        return true;
    }

    let path_text = path.to_string_lossy().to_ascii_lowercase();
    if CRITICAL_EXACT_FILES
        .iter()
        .any(|critical| path_text == *critical)
        || CRITICAL_FILE_PREFIXES
            .iter()
            .any(|prefix| path_text.starts_with(prefix))
    {
        return true;
    }

    let in_sensitive_area = SENSITIVE_AREAS
        .iter()
        .map(Path::new)
        .any(|root| path.starts_with(root));
    in_sensitive_area
        && SENSITIVE_IDENTIFIERS
            .iter()
            .any(|identifier| path_text.contains(identifier))
}

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::symlink};

    use tempfile::TempDir;

    use super::*;

    fn empty_rules() -> ModuleRules {
        ModuleRules {
            default_mode: MountMode::Overlay,
            ..Default::default()
        }
    }

    fn rules_with_ignore(path: &str) -> ModuleRules {
        let mut rules = empty_rules();
        rules.paths.insert(path.to_string(), MountMode::Ignore);
        rules
    }

    #[test]
    fn recognizes_firmware_radio_and_vendor_control_roots() {
        for path in [
            "vendor/firmware/image.mbn",
            "vendor/bt_firmware/image.bin",
            "vendor/rfs/msm/mpss/readonly/firmware",
            "vendor/etc/init/hw/init.qcom.rc",
            "vendor/etc/vintf/manifest.xml",
            "vendor/etc/selinux/vendor_sepolicy.cil",
            "odm/etc/bluetooth/bt_vendor.conf",
            "my_carrier/etc/radio/config.xml",
        ] {
            assert!(is_critical_payload_path(Path::new(path)), "{path}");
        }
    }

    #[test]
    fn recognizes_partition_aliases_and_sensitive_hal_names() {
        assert_eq!(
            normalize_partition_alias(Path::new("system/vendor/firmware/modem.mbn")),
            PathBuf::from("vendor/firmware/modem.mbn")
        );
        for path in [
            "vendor/bin/hw/android.hardware.radio-service",
            "vendor/lib64/vendor.oplus.hardware.bluetooth-V1-ndk.so",
            "vendor/lib64/libbt-vendor.so",
            "vendor/etc/permissions/android.hardware.telephony.xml",
        ] {
            assert!(is_critical_payload_path(Path::new(path)), "{path}");
        }
    }

    #[test]
    fn recognizes_fstab_build_property_and_partition_replacement_controls() {
        for path in [
            "vendor/etc/fstab.qcom",
            "vendor/build.prop",
            "odm/ueventd.rc",
        ] {
            assert!(is_critical_payload_path(Path::new(path)), "{path}");
        }
        for path in [
            "system/.replace",
            "vendor/.replace",
            "vendor/etc/.replace",
            "vendor/lib64/.replace",
            "odm/.replace",
            "my_carrier/etc/.replace",
        ] {
            assert!(is_critical_replace_marker(Path::new(path)), "{path}");
        }
    }

    #[test]
    fn unrelated_audio_and_application_paths_remain_allowed() {
        for path in [
            "vendor/lib64/soundfx/libdolby.so",
            "vendor/etc/audio_effects.xml",
            "system/app/example.apk",
            "product/overlay/theme.apk",
        ] {
            assert!(!is_critical_payload_path(Path::new(path)), "{path}");
        }
    }

    #[test]
    fn blocks_critical_directory_without_following_symlink() {
        let temp = TempDir::new().unwrap();
        let module = temp.path().join("module");
        fs::create_dir_all(module.join("vendor")).unwrap();
        symlink("/outside/firmware", module.join("vendor/firmware")).unwrap();

        assert_eq!(
            first_blocked_critical_path(&module, &empty_rules()).unwrap(),
            Some(PathBuf::from("vendor/firmware"))
        );
    }

    #[test]
    fn blocks_partition_root_symlink_even_when_root_rule_is_ignore() {
        let temp = TempDir::new().unwrap();
        let module = temp.path().join("module");
        fs::create_dir_all(&module).unwrap();
        symlink("/outside/vendor", module.join("vendor")).unwrap();

        assert_eq!(
            first_blocked_critical_path(&module, &rules_with_ignore("vendor")).unwrap(),
            Some(PathBuf::from("vendor"))
        );
    }

    #[test]
    fn blocks_partition_root_replace_marker() {
        let temp = TempDir::new().unwrap();
        let module = temp.path().join("module");
        fs::create_dir_all(module.join("vendor")).unwrap();
        fs::write(module.join("vendor/.replace"), b"").unwrap();

        assert_eq!(
            first_blocked_critical_path(&module, &empty_rules()).unwrap(),
            Some(PathBuf::from("vendor/.replace"))
        );
    }

    #[test]
    fn replace_marker_requires_parent_directory_ignore() {
        let temp = TempDir::new().unwrap();
        let module = temp.path().join("module");
        fs::create_dir_all(module.join("vendor")).unwrap();
        fs::write(module.join("vendor/.replace"), b"").unwrap();

        assert_eq!(
            first_blocked_critical_path(&module, &rules_with_ignore("vendor/.replace")).unwrap(),
            Some(PathBuf::from("vendor/.replace"))
        );
        assert_eq!(
            first_blocked_critical_path(&module, &rules_with_ignore("vendor")).unwrap(),
            None
        );
    }

    #[test]
    fn sensitive_file_requires_parent_directory_ignore() {
        let temp = TempDir::new().unwrap();
        let module = temp.path().join("module");
        fs::create_dir_all(module.join("vendor/lib64")).unwrap();
        fs::write(module.join("vendor/lib64/libbt-vendor.so"), b"blocked").unwrap();

        assert_eq!(
            first_blocked_critical_path(
                &module,
                &rules_with_ignore("vendor/lib64/libbt-vendor.so")
            )
            .unwrap(),
            Some(PathBuf::from("vendor/lib64/libbt-vendor.so"))
        );
        assert_eq!(
            first_blocked_critical_path(&module, &rules_with_ignore("vendor/lib64")).unwrap(),
            None
        );
    }

    #[test]
    fn explicit_directory_ignore_allows_safe_remainder_of_module() {
        let temp = TempDir::new().unwrap();
        let module = temp.path().join("module");
        fs::create_dir_all(module.join("vendor/firmware")).unwrap();
        fs::write(module.join("vendor/firmware/modem.mbn"), b"blocked").unwrap();
        fs::create_dir_all(module.join("system/app")).unwrap();
        fs::write(module.join("system/app/example.apk"), b"allowed").unwrap();

        assert_eq!(
            first_blocked_critical_path(&module, &rules_with_ignore("vendor/firmware")).unwrap(),
            None
        );
    }

    #[test]
    fn partition_alias_directory_ignore_is_honored() {
        let temp = TempDir::new().unwrap();
        let module = temp.path().join("module");
        fs::create_dir_all(module.join("system/vendor/firmware")).unwrap();
        fs::write(module.join("system/vendor/firmware/modem.mbn"), b"blocked").unwrap();

        assert_eq!(
            first_blocked_critical_path(&module, &rules_with_ignore("system/vendor/firmware"))
                .unwrap(),
            None
        );
    }
}
