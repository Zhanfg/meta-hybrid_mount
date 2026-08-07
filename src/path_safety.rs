// Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-only

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use serde::{Deserialize, Deserializer, de::Error as _};

const CRITICAL_SYSTEM_TREES: &[&str] = &[
    "/system/etc/firmware",
    "/vendor/firmware",
    "/vendor/firmware_mnt",
    "/vendor/bt_firmware",
    "/vendor/etc/firmware",
    "/vendor/rfs",
    "/vendor/etc/bluetooth",
    "/vendor/etc/qcril_database",
    "/vendor/etc/radio",
    "/vendor/etc/modem",
    "/vendor/etc/init",
    "/vendor/etc/vintf",
    "/vendor/etc/selinux",
    "/odm/firmware",
    "/odm/bt_firmware",
    "/odm/etc/firmware",
    "/odm/etc/bluetooth",
    "/odm/etc/radio",
    "/odm/etc/modem",
    "/odm/etc/init",
    "/odm/etc/vintf",
    "/odm/etc/selinux",
    "/my_carrier/firmware",
    "/my_carrier/bt_firmware",
    "/my_carrier/etc/firmware",
    "/my_carrier/etc/radio",
    "/my_carrier/etc/modem",
    "/my_carrier/etc/init",
    "/my_carrier/etc/vintf",
];

const SENSITIVE_SYSTEM_AREAS: &[&str] = &[
    "/vendor/bin",
    "/vendor/lib",
    "/vendor/lib64",
    "/vendor/etc/permissions",
    "/vendor/etc/sysconfig",
    "/odm/bin",
    "/odm/lib",
    "/odm/lib64",
    "/odm/etc/permissions",
    "/odm/etc/sysconfig",
    "/my_carrier/bin",
    "/my_carrier/lib",
    "/my_carrier/lib64",
    "/my_carrier/etc/permissions",
    "/my_carrier/etc/sysconfig",
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
    "/vendor/ueventd.rc",
    "/vendor/build.prop",
    "/vendor/default.prop",
    "/odm/ueventd.rc",
    "/odm/build.prop",
    "/my_carrier/build.prop",
];

const CRITICAL_FILE_PREFIXES: &[&str] = &[
    "/vendor/etc/fstab.",
    "/odm/etc/fstab.",
    "/my_carrier/etc/fstab.",
];

fn normalize_managed_alias_text(target: &Path) -> String {
    let text = target.to_string_lossy().to_ascii_lowercase();
    for (alias, canonical) in [
        ("/system/vendor", "/vendor"),
        ("/system/odm", "/odm"),
        ("/system/product", "/product"),
        ("/system/system_ext", "/system_ext"),
    ] {
        if text == alias {
            return canonical.to_string();
        }
        if let Some(rest) = text.strip_prefix(alias)
            && rest.starts_with('/')
        {
            return format!("{canonical}{rest}");
        }
    }
    text
}

pub fn is_radio_critical_system_path(target: &Path) -> bool {
    let text = normalize_managed_alias_text(target);

    if [
        "/system",
        "/vendor",
        "/odm",
        "/product",
        "/system_ext",
        "/my_carrier",
    ]
    .contains(&text.as_str())
    {
        return true;
    }

    if CRITICAL_SYSTEM_TREES
        .iter()
        .any(|root| text == *root || text.starts_with(&format!("{root}/")))
    {
        return true;
    }

    if CRITICAL_EXACT_FILES
        .iter()
        .any(|critical| text == *critical)
        || CRITICAL_FILE_PREFIXES
            .iter()
            .any(|prefix| text.starts_with(prefix))
    {
        return true;
    }

    let in_sensitive_area = SENSITIVE_SYSTEM_AREAS
        .iter()
        .any(|root| text == *root || text.starts_with(&format!("{root}/")));
    in_sensitive_area
        && SENSITIVE_IDENTIFIERS
            .iter()
            .any(|identifier| text.contains(identifier))
}

pub fn ensure_kasumi_target_allowed(target: &Path) -> Result<()> {
    if !target.is_absolute() {
        bail!("Kasumi target must be an absolute path: {}", target.display());
    }
    let normalized = crate::utils::normalize_path(target);
    if normalized != target {
        bail!(
            "Kasumi target must already be normalized: original={}, normalized={}",
            target.display(),
            normalized.display()
        );
    }
    if target == Path::new("/") {
        bail!("refusing a Kasumi rule targeting the filesystem root");
    }
    if is_radio_critical_system_path(target) {
        bail!(
            "Kasumi target is inside a protected radio/firmware/system-critical path: {}",
            target.display()
        );
    }
    Ok(())
}

pub fn deserialize_safe_kasumi_target<'de, D>(deserializer: D) -> Result<PathBuf, D::Error>
where
    D: Deserializer<'de>,
{
    let path = PathBuf::deserialize(deserializer)?;
    ensure_kasumi_target_allowed(&path).map_err(D::Error::custom)?;
    Ok(path)
}

pub fn deserialize_optional_safe_kasumi_target<'de, D>(
    deserializer: D,
) -> Result<PathBuf, D::Error>
where
    D: Deserializer<'de>,
{
    let path = PathBuf::deserialize(deserializer)?;
    if !path.as_os_str().is_empty() {
        ensure_kasumi_target_allowed(&path).map_err(D::Error::custom)?;
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_radio_and_firmware_targets_with_managed_aliases() {
        for path in [
            "/vendor/firmware/modem.mbn",
            "/system/vendor/etc/bluetooth/bt_vendor.conf",
            "/vendor/lib64/libbt-vendor.so",
            "/vendor/etc/init/hw/init.qcom.rc",
            "/odm/etc/vintf/manifest.xml",
            "/my_carrier/etc/modem/config.xml",
        ] {
            assert!(ensure_kasumi_target_allowed(Path::new(path)).is_err(), "{path}");
        }
    }

    #[test]
    fn leaves_normal_module_and_data_targets_available() {
        for path in [
            "/system/app/Example/Example.apk",
            "/vendor/lib64/soundfx/libdolby.so",
            "/vendor/etc/audio_effects.xml",
            "/data/adb/modules/example",
        ] {
            assert!(ensure_kasumi_target_allowed(Path::new(path)).is_ok(), "{path}");
        }
    }

    #[test]
    fn rejects_root_relative_and_non_normalized_targets() {
        assert!(ensure_kasumi_target_allowed(Path::new("/")).is_err());
        assert!(ensure_kasumi_target_allowed(Path::new("vendor/etc/radio")).is_err());
        assert!(ensure_kasumi_target_allowed(Path::new("/system/../vendor/etc/radio")).is_err());
    }
}
