// Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-only

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
#[cfg(any(target_os = "linux", target_os = "android"))]
use rustix::mount::{MountFlags, UnmountFlags, mount_bind, mount_remount, unmount};

use crate::conf::schema::CustomBindMount;

const CRITICAL_BIND_TREES: &[&str] = &[
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

const SENSITIVE_BIND_AREAS: &[&str] = &[
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

const SENSITIVE_BIND_IDENTIFIERS: &[&str] = &[
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

const CRITICAL_BIND_EXACT_FILES: &[&str] = &[
    "/vendor/ueventd.rc",
    "/vendor/build.prop",
    "/vendor/default.prop",
    "/odm/ueventd.rc",
    "/odm/build.prop",
    "/my_carrier/build.prop",
];

const CRITICAL_BIND_FILE_PREFIXES: &[&str] = &[
    "/vendor/etc/fstab.",
    "/odm/etc/fstab.",
    "/my_carrier/etc/fstab.",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CustomBindKind {
    File,
    Directory,
}

#[derive(Debug, Clone)]
pub struct AppliedCustomBind {
    pub source: PathBuf,
    pub target: PathBuf,
    pub kind: CustomBindKind,
}

pub fn apply_custom_bind_mounts(
    mounts: &[CustomBindMount],
    disable_umount: bool,
) -> Result<Vec<AppliedCustomBind>> {
    let mut applied_mounts = Vec::with_capacity(mounts.len());

    for mount in mounts {
        let applied = match apply_one(mount, disable_umount).with_context(|| {
            format!(
                "failed to apply custom bind {} -> {}",
                mount.source.display(),
                mount.target.display()
            )
        }) {
            Ok(applied) => applied,
            Err(error) => {
                let rollback_errors = rollback_applied_mounts(&applied_mounts);
                if rollback_errors.is_empty() {
                    return Err(error);
                }
                bail!(
                    "custom bind application failed and previous mounts could not be fully rolled back: error={:#}; rollback_errors={}",
                    error,
                    rollback_errors.join(" | ")
                );
            }
        };
        crate::scoped_log!(
            info,
            "custom_bind",
            "mounted: source={}, target={}",
            applied.source.display(),
            applied.target.display()
        );
        applied_mounts.push(applied);
    }

    Ok(applied_mounts)
}

fn apply_one(mount: &CustomBindMount, disable_umount: bool) -> Result<AppliedCustomBind> {
    let kind = validate_mount_paths(&mount.source, &mount.target)?;
    bind_mount_checked(&mount.source, &mount.target, disable_umount)?;

    Ok(AppliedCustomBind {
        source: mount.source.clone(),
        target: mount.target.clone(),
        kind,
    })
}

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

fn target_is_critical_system_path(target: &Path) -> bool {
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

    if CRITICAL_BIND_TREES
        .iter()
        .any(|root| text == *root || text.starts_with(&format!("{root}/")))
    {
        return true;
    }

    if CRITICAL_BIND_EXACT_FILES
        .iter()
        .any(|critical| text == *critical)
        || CRITICAL_BIND_FILE_PREFIXES
            .iter()
            .any(|prefix| text.starts_with(prefix))
    {
        return true;
    }

    let in_sensitive_area = SENSITIVE_BIND_AREAS
        .iter()
        .any(|root| text == *root || text.starts_with(&format!("{root}/")));
    in_sensitive_area
        && SENSITIVE_BIND_IDENTIFIERS
            .iter()
            .any(|identifier| text.contains(identifier))
}

fn target_is_forbidden(target: &Path) -> bool {
    target == Path::new("/")
        || [
            "/proc",
            "/sys",
            "/dev",
            "/mnt",
            "/storage",
            "/data/adb",
            "/apex",
        ]
        .into_iter()
        .any(|root| target.starts_with(root))
        || target_is_critical_system_path(target)
}

fn ensure_target_allowed(target: &Path) -> Result<()> {
    if target_is_forbidden(target) {
        bail!(
            "custom bind target is inside a protected runtime or radio-critical path: {}",
            target.display()
        );
    }
    Ok(())
}

fn validate_mount_paths(source: &Path, target: &Path) -> Result<CustomBindKind> {
    if !source.is_absolute() {
        bail!("custom bind source must be an absolute path");
    }
    if !target.is_absolute() {
        bail!("custom bind target must be an absolute path");
    }
    if source == target {
        bail!("custom bind source and target must differ");
    }
    ensure_target_allowed(target)?;

    let source_meta = fs::metadata(source)
        .with_context(|| format!("failed to inspect source {}", source.display()))?;
    let target_meta = fs::metadata(target)
        .with_context(|| format!("failed to inspect target {}", target.display()))?;
    let source_real = fs::canonicalize(source)
        .with_context(|| format!("failed to resolve source {}", source.display()))?;
    let target_real = fs::canonicalize(target)
        .with_context(|| format!("failed to resolve target {}", target.display()))?;
    ensure_target_allowed(&target_real)?;

    let source_type = source_meta.file_type();
    let target_type = target_meta.file_type();
    if !(source_type.is_dir() || source_type.is_file()) {
        bail!("custom bind source must be a regular file or directory");
    }
    if !(target_type.is_dir() || target_type.is_file()) {
        bail!("custom bind target must be a regular file or directory");
    }

    match (source_meta.is_dir(), target_meta.is_dir()) {
        (true, true) => {
            if source_real == target_real
                || source_real.starts_with(&target_real)
                || target_real.starts_with(&source_real)
            {
                bail!(
                    "custom bind directory trees must not overlap: {} -> {}",
                    source.display(),
                    target.display()
                );
            }
            Ok(CustomBindKind::Directory)
        }
        (false, false) => {
            if source_real == target_real {
                bail!("custom bind source and target resolve to the same file");
            }
            Ok(CustomBindKind::File)
        }
        (true, false) => bail!("custom bind source is a directory but target is not"),
        (false, true) => bail!("custom bind source is not a directory but target is"),
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn bind_mount_checked(source: &Path, target: &Path, disable_umount: bool) -> Result<()> {
    mount_bind(source, target).with_context(|| {
        format!(
            "failed to bind mount {} to {}",
            source.display(),
            target.display()
        )
    })?;

    if let Err(error) = mount_remount(target, MountFlags::RDONLY | MountFlags::BIND, "") {
        let cleanup_error = unmount(target, UnmountFlags::DETACH).err();
        if let Some(cleanup_error) = cleanup_error {
            bail!(
                "failed to remount custom bind readonly and rollback failed for {}: remount={:#}; rollback={:#}",
                target.display(),
                error,
                cleanup_error
            );
        }
        return Err(error).with_context(|| {
            format!(
                "failed to remount custom bind readonly: {}",
                target.display()
            )
        });
    }

    if !disable_umount && let Err(error) = crate::mount::umount_mgr::send_umountable(target) {
        crate::scoped_log!(
            warn,
            "custom_bind",
            "bind mounted but umount registration failed: target={}, error={:#}",
            target.display(),
            error
        );
    }

    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn rollback_applied_mounts(mounts: &[AppliedCustomBind]) -> Vec<String> {
    mounts
        .iter()
        .rev()
        .filter_map(|mount| {
            unmount(&mount.target, UnmountFlags::DETACH)
                .err()
                .map(|error| format!("{}: {error:#}", mount.target.display()))
        })
        .collect()
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn rollback_applied_mounts(_mounts: &[AppliedCustomBind]) -> Vec<String> {
    Vec::new()
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn bind_mount_checked(_source: &Path, _target: &Path, _disable_umount: bool) -> Result<()> {
    bail!("custom bind mounts are only supported on linux/android")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_rejects_relative_paths() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target");
        fs::write(&target, b"").unwrap();

        assert!(validate_mount_paths(Path::new("relative"), &target).is_err());
        assert!(validate_mount_paths(&target, Path::new("relative")).is_err());
    }

    #[test]
    fn validate_rejects_protected_targets_before_filesystem_access() {
        for target in [
            "/",
            "/data/adb/modules/hybrid_mount",
            "/proc/sys",
            "/apex/com.android.runtime",
            "/vendor/firmware/modem.mbn",
            "/system/vendor/etc/bluetooth/bt_vendor.conf",
            "/vendor/lib64/libbt-vendor.so",
            "/vendor/etc/init/hw/init.qcom.rc",
            "/odm/etc/vintf/manifest.xml",
        ] {
            assert!(
                validate_mount_paths(Path::new("/missing-source"), Path::new(target)).is_err(),
                "{target}"
            );
        }
    }

    #[test]
    fn radio_critical_filter_leaves_audio_payload_targets_available() {
        assert!(!target_is_critical_system_path(Path::new(
            "/vendor/lib64/soundfx/libdolby.so"
        )));
        assert!(!target_is_critical_system_path(Path::new(
            "/vendor/etc/audio_effects.xml"
        )));
        assert!(target_is_critical_system_path(Path::new(
            "/vendor/lib64/libbt-vendor.so"
        )));
    }

    #[cfg(unix)]
    #[test]
    fn validate_rejects_symlink_into_protected_target() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        fs::create_dir(&source).unwrap();
        symlink("/proc", &target).unwrap();

        assert!(validate_mount_paths(&source, &target).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn validate_rejects_special_file_source() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target");
        fs::write(&target, b"").unwrap();

        assert!(validate_mount_paths(Path::new("/dev/null"), &target).is_err());
    }

    #[test]
    fn validate_rejects_type_mismatch() {
        let temp = tempfile::tempdir().unwrap();
        let source_dir = temp.path().join("source");
        let target_file = temp.path().join("target");
        fs::create_dir(&source_dir).unwrap();
        fs::write(&target_file, b"").unwrap();

        assert!(validate_mount_paths(&source_dir, &target_file).is_err());
    }

    #[test]
    fn validate_rejects_overlapping_directory_trees() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let target = source.join("target");
        fs::create_dir_all(&target).unwrap();

        assert!(validate_mount_paths(&source, &target).is_err());
        assert!(validate_mount_paths(&target, &source).is_err());
    }

    #[test]
    fn validate_accepts_file_to_file() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        fs::write(&source, b"").unwrap();
        fs::write(&target, b"").unwrap();

        assert_eq!(
            validate_mount_paths(&source, &target).unwrap(),
            CustomBindKind::File
        );
    }

    #[test]
    fn validate_accepts_dir_to_dir() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&target).unwrap();

        assert_eq!(
            validate_mount_paths(&source, &target).unwrap(),
            CustomBindKind::Directory
        );
    }
}
