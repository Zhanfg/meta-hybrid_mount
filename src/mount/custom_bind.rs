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

fn target_is_forbidden(target: &Path) -> bool {
    target == Path::new("/")
        || ["/proc", "/sys", "/dev", "/mnt", "/storage", "/data/adb"]
            .into_iter()
            .any(|root| target.starts_with(root))
}

fn ensure_target_allowed(target: &Path) -> Result<()> {
    if target_is_forbidden(target) {
        bail!(
            "custom bind target is inside a protected runtime root: {}",
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
        assert!(validate_mount_paths(Path::new("/missing-source"), Path::new("/")).is_err());
        assert!(
            validate_mount_paths(
                Path::new("/missing-source"),
                Path::new("/data/adb/modules/hybrid_mount")
            )
            .is_err()
        );
        assert!(
            validate_mount_paths(Path::new("/missing-source"), Path::new("/proc/sys"))
                .is_err()
        );
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
