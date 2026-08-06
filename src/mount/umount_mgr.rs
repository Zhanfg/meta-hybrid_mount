// Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-only

use std::path::{Path, PathBuf};
#[cfg(any(target_os = "linux", target_os = "android"))]
use std::{
    collections::HashSet,
    sync::{LazyLock, Mutex},
};

#[cfg(any(target_os = "linux", target_os = "android"))]
use anyhow::Context;
use anyhow::Result;
#[cfg(any(target_os = "linux", target_os = "android"))]
use ksu::{TryUmount, TryUmountFlags};
#[cfg(any(target_os = "linux", target_os = "android"))]
use procfs::process::Process;

#[cfg(any(target_os = "linux", target_os = "android"))]
static PENDING: LazyLock<Mutex<HashSet<PathBuf>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

pub fn send_umountable<P>(target: P) -> Result<()>
where
    P: AsRef<Path>,
{
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        let _ = target;
        Ok(())
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        if !crate::utils::KSU.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok(());
        }

        PENDING
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to lock pending umount targets"))?
            .insert(target.as_ref().to_path_buf());
        Ok(())
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
pub fn commit() -> Result<()> {
    if !crate::utils::KSU.load(std::sync::atomic::Ordering::Relaxed) {
        return Ok(());
    }

    let mounted = Process::myself()?
        .mountinfo()?
        .into_iter()
        .map(|entry| entry.mount_point)
        .collect::<HashSet<_>>();
    let mut pending = PENDING
        .lock()
        .map_err(|_| anyhow::anyhow!("Failed to lock pending umount targets"))?;
    retain_current_mounts(&mut pending, &mounted);

    if pending.is_empty() {
        crate::scoped_log!(
            debug,
            "umount_mgr",
            "commit skipped: no current mount targets"
        );
        return Ok(());
    }

    let mut list = TryUmount::new();
    for target in pending.iter() {
        list.add(target);
    }
    list.format_msg(|path| format!("{path:?} umount successful "));
    list.flags(TryUmountFlags::MNT_DETACH);
    list.umount().context("KernelSU try-umount commit failed")?;

    pending.clear();
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn retain_current_mounts(pending: &mut HashSet<PathBuf>, mounted: &HashSet<PathBuf>) {
    let stale = pending
        .iter()
        .filter(|path| !mounted.contains(*path))
        .cloned()
        .collect::<Vec<_>>();
    for path in &stale {
        crate::scoped_log!(
            debug,
            "umount_mgr",
            "discard stale target after mount move: path={}",
            path.display()
        );
    }
    pending.retain(|path| mounted.contains(path));
}

#[cfg(all(test, any(target_os = "linux", target_os = "android")))]
mod tests {
    use super::*;

    #[test]
    fn moved_magic_children_are_removed_but_final_parent_is_kept() {
        let mut pending = HashSet::from([
            PathBuf::from("/mnt/work/system/app/example.apk"),
            PathBuf::from("/system/app"),
        ]);
        let mounted = HashSet::from([
            PathBuf::from("/system/app"),
            PathBuf::from("/system/app/example.apk"),
        ]);

        retain_current_mounts(&mut pending, &mounted);

        assert_eq!(pending, HashSet::from([PathBuf::from("/system/app")]));
    }
}
