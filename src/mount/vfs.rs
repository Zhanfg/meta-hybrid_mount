// Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-only

#[cfg(any(target_os = "linux", target_os = "android"))]
use std::ffi::CString;
use std::path::Path;

#[cfg(any(target_os = "linux", target_os = "android"))]
use anyhow::Context;
use anyhow::{Result, bail};
#[cfg(any(target_os = "linux", target_os = "android"))]
use rustix::mount::{MountFlags, UnmountFlags, mount, unmount};

use crate::core::ops::plan::VfsOperation;

pub fn mount_union(op: &VfsOperation) -> Result<()> {
    validate_operation(op)?;

    let mut branches = op
        .lowerdirs
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    branches.push(op.target.to_string_lossy().into_owned());
    let option = format!("lowerdir={}", branches.join(":"));

    mount_vfs(op, &option)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn mount_vfs(op: &VfsOperation, option: &str) -> Result<()> {
    let data = CString::new(option).context("VFS mount options contain an interior NUL")?;
    mount(
        "none",
        &op.target,
        op.backend.as_str(),
        MountFlags::empty(),
        Some(data.as_c_str()),
    )
    .with_context(|| {
        format!(
            "{} mount syscall failed for {}",
            op.backend,
            op.target.display()
        )
    })?;
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn mount_vfs(op: &VfsOperation, _option: &str) -> Result<()> {
    bail!(
        "{} VFS mounting is only supported on linux/android",
        op.backend
    )
}

pub fn rollback_targets(targets: &[std::path::PathBuf]) -> Result<()> {
    let mut failures = Vec::new();
    for target in targets.iter().rev() {
        if let Err(error) = detach(target) {
            failures.push(format!("{}: {error:#}", target.display()));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        bail!("VFS rollback incomplete: {}", failures.join(" | "))
    }
}

fn validate_operation(op: &VfsOperation) -> Result<()> {
    if !matches!(op.backend.as_str(), "mirage" | "nomountfs") {
        bail!("unsupported VFS filesystem type: {}", op.backend);
    }
    if op.max_branches < 2 || op.max_branches > 5 {
        bail!("unsafe VFS branch limit: {}", op.max_branches);
    }
    if op.lowerdirs.is_empty() {
        bail!("VFS operation has no module lowerdirs");
    }
    if op.lowerdirs.len() + 1 > op.max_branches {
        bail!(
            "VFS branch limit exceeded: module_branches={}, physical_branch=1, max={}",
            op.lowerdirs.len(),
            op.max_branches
        );
    }
    validate_path(&op.target, "target")?;
    if !op.target.is_dir() {
        bail!("VFS target is not a directory: {}", op.target.display());
    }
    for lowerdir in &op.lowerdirs {
        validate_path(lowerdir, "lowerdir")?;
        if !lowerdir.is_dir() {
            bail!("VFS lowerdir is not a directory: {}", lowerdir.display());
        }
    }
    Ok(())
}

fn validate_path(path: &Path, role: &str) -> Result<()> {
    if !path.is_absolute() {
        bail!("VFS {role} must be absolute: {}", path.display());
    }
    if path.to_string_lossy().contains(':') {
        bail!(
            "VFS {role} contains unsupported ':' separator: {}",
            path.display()
        );
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn detach(target: &Path) -> Result<()> {
    match unmount(target, UnmountFlags::DETACH) {
        Ok(()) => Ok(()),
        Err(rustix::io::Errno::INVAL | rustix::io::Errno::NOENT) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn detach(_target: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn rejects_mirage_branch_overflow_before_mount() {
        let op = VfsOperation {
            backend: "mirage".to_string(),
            partition_name: "system".to_string(),
            target: PathBuf::from("/system"),
            lowerdirs: (0..5)
                .map(|idx| PathBuf::from(format!("/data/adb/modules/m{idx}/system")))
                .collect(),
            module_ids: Vec::new(),
            max_branches: 5,
        };
        assert!(validate_operation(&op).is_err());
    }

    #[test]
    fn rejects_unverified_filesystem_names() {
        let op = VfsOperation {
            backend: "zeromount".to_string(),
            partition_name: "system".to_string(),
            target: PathBuf::from("/system"),
            lowerdirs: vec![PathBuf::from("/data/adb/modules/a/system")],
            module_ids: vec!["a".to_string()],
            max_branches: 5,
        };
        assert!(validate_operation(&op).is_err());
    }

    #[test]
    fn rejects_colon_in_lowerdir_before_option_serialization() {
        let op = VfsOperation {
            backend: "mirage".to_string(),
            partition_name: "system".to_string(),
            target: PathBuf::from("/system"),
            lowerdirs: vec![PathBuf::from("/data/adb/modules/bad:module/system")],
            module_ids: vec!["bad".to_string()],
            max_branches: 5,
        };
        assert!(validate_operation(&op).is_err());
    }
}
