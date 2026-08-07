// Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-only

use std::{
    collections::VecDeque,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};

use crate::domain::{ModuleRules, MountMode};

const MAX_SCANNED_ENTRIES: usize = 100_000;
const MAX_DIRECTORY_DEPTH: usize = 64;

const CRITICAL_TREE_ROOTS: &[&str] = &[
    "system/etc/firmware",
    "vendor/firmware",
    "vendor/firmware_mnt",
    "vendor/rfs",
    "vendor/etc/bluetooth",
    "vendor/etc/qcril_database",
    "vendor/etc/radio",
    "vendor/etc/modem",
    "odm/firmware",
    "odm/etc/bluetooth",
    "odm/etc/radio",
    "odm/etc/modem",
    "my_carrier/firmware",
    "my_carrier/etc/radio",
    "my_carrier/etc/modem",
];

const SENSITIVE_AREAS: &[&str] = &[
    "vendor/bin",
    "vendor/lib",
    "vendor/lib64",
    "vendor/etc/init",
    "vendor/etc/vintf",
    "odm/bin",
    "odm/lib",
    "odm/lib64",
    "odm/etc/init",
    "odm/etc/vintf",
    "my_carrier/bin",
    "my_carrier/lib",
    "my_carrier/lib64",
    "my_carrier/etc/init",
];

const SENSITIVE_IDENTIFIERS: &[&str] = &[
    "android.hardware.bluetooth",
    "android.hardware.radio",
    "vendor.qti.hardware.bluetooth",
    "vendor.qti.hardware.radio",
    "vendor.oplus.hardware.bluetooth",
    "vendor.oplus.hardware.radio",
    "qcril",
    "libril",
    "/rild",
    "modem_service",
    "modemservice",
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
                bail!(
                    "module tree exceeds critical-path scan entry limit ({MAX_SCANNED_ENTRIES})"
                );
            }

            let entry = entry.with_context(|| {
                format!("failed to enumerate module path {}", directory.display())
            })?;
            let relative_path = relative_directory.join(entry.file_name());
            let normalized_path = normalize_partition_alias(&relative_path);

            if is_critical_payload_path(&normalized_path) {
                let ignored = matches!(
                    rules.effective_mode(&relative_path),
                    MountMode::Ignore
                ) || (normalized_path != relative_path
                    && matches!(
                        rules.effective_mode(&normalized_path),
                        MountMode::Ignore
                    ));
                if ignored {
                    crate::scoped_log!(
                        warn,
                        "inventory:safety",
                        "critical payload ignored by explicit rule: path={}",
                        relative_path.display()
                    );
                    continue;
                }
                return Ok(Some(relative_path));
            }

            let file_type = entry
                .file_type()
                .with_context(|| format!("failed to inspect {}", entry.path().display()))?;
            if file_type.is_dir() {
                queue.push_back((entry.path(), relative_path, depth + 1));
            }
        }
    }

    Ok(None)
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

fn is_critical_payload_path(path: &Path) -> bool {
    if CRITICAL_TREE_ROOTS
        .iter()
        .map(Path::new)
        .any(|root| path == root || path.starts_with(root))
    {
        return true;
    }

    let in_sensitive_area = SENSITIVE_AREAS
        .iter()
        .map(Path::new)
        .any(|root| path.starts_with(root));
    if !in_sensitive_area {
        return false;
    }

    let path_text = path.to_string_lossy().to_ascii_lowercase();
    SENSITIVE_IDENTIFIERS
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

    #[test]
    fn recognizes_firmware_and_radio_roots() {
        for path in [
            "vendor/firmware/image.mbn",
            "vendor/rfs/msm/mpss/readonly/firmware",
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
        assert!(is_critical_payload_path(Path::new(
            "vendor/bin/hw/android.hardware.radio-service"
        )));
        assert!(is_critical_payload_path(Path::new(
            "vendor/lib64/vendor.oplus.hardware.bluetooth-V1-ndk.so"
        )));
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
    fn explicit_ignore_rule_allows_safe_remainder_of_module() {
        let temp = TempDir::new().unwrap();
        let module = temp.path().join("module");
        fs::create_dir_all(module.join("vendor/firmware")).unwrap();
        fs::write(module.join("vendor/firmware/modem.mbn"), b"blocked").unwrap();
        fs::create_dir_all(module.join("system/app")).unwrap();
        fs::write(module.join("system/app/example.apk"), b"allowed").unwrap();

        let mut rules = empty_rules();
        rules
            .paths
            .insert("vendor/firmware".to_string(), MountMode::Ignore);

        assert_eq!(
            first_blocked_critical_path(&module, &rules).unwrap(),
            None
        );
    }

    #[test]
    fn partition_alias_ignore_rule_is_honored() {
        let temp = TempDir::new().unwrap();
        let module = temp.path().join("module");
        fs::create_dir_all(module.join("system/vendor/firmware")).unwrap();
        fs::write(
            module.join("system/vendor/firmware/modem.mbn"),
            b"blocked",
        )
        .unwrap();

        let mut rules = empty_rules();
        rules.paths.insert(
            "system/vendor/firmware".to_string(),
            MountMode::Ignore,
        );

        assert_eq!(
            first_blocked_critical_path(&module, &rules).unwrap(),
            None
        );
    }
}
