// Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-only

use std::path::{Path, PathBuf};
#[cfg(any(target_os = "linux", target_os = "android"))]
use std::{
    collections::HashSet,
    ffi::CString,
    fs,
    os::{fd::RawFd, unix::ffi::OsStrExt},
    sync::{LazyLock, Mutex, OnceLock},
};

#[cfg(any(target_os = "linux", target_os = "android"))]
use anyhow::Context;
use anyhow::{Result, bail};
#[cfg(any(target_os = "linux", target_os = "android"))]
use ksu::{TryUmount, TryUmountFlags};
#[cfg(any(target_os = "linux", target_os = "android"))]
use procfs::process::Process;

#[cfg(any(target_os = "linux", target_os = "android"))]
static PENDING: LazyLock<Mutex<HashSet<PathBuf>>> = LazyLock::new(|| Mutex::new(HashSet::new()));
#[cfg(any(target_os = "linux", target_os = "android"))]
static KSU_DRIVER_FD: OnceLock<RawFd> = OnceLock::new();

#[cfg(any(target_os = "linux", target_os = "android"))]
const KSU_IOCTL_MAGIC: u32 = b'K' as u32;
#[cfg(any(target_os = "linux", target_os = "android"))]
const KSU_INSTALL_MAGIC1: u32 = 0xDEADBEEF;
#[cfg(any(target_os = "linux", target_os = "android"))]
const KSU_INSTALL_MAGIC2: u32 = 0xCAFEBABE;
#[cfg(all(any(target_os = "linux", target_os = "android"), target_env = "gnu"))]
const KSU_IOCTL_ADD_TRY_UMOUNT: u64 = libc::_IOW::<()>(KSU_IOCTL_MAGIC, 18);
#[cfg(all(
    any(target_os = "linux", target_os = "android"),
    not(target_env = "gnu")
))]
const KSU_IOCTL_ADD_TRY_UMOUNT: i32 = libc::_IOW::<()>(KSU_IOCTL_MAGIC, 18);

#[cfg(any(target_os = "linux", target_os = "android"))]
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct AddTryUmountCmd {
    arg: u64,
    flags: u32,
    mode: u8,
}

#[cfg(any(target_os = "linux", target_os = "android"))]
impl AddTryUmountCmd {
    fn delete(path_ptr: u64) -> Self {
        Self {
            arg: path_ptr,
            flags: 0,
            mode: 2,
        }
    }
}

#[derive(Debug, Default)]
pub struct UmountRegistrationGuard {
    registered: Vec<PathBuf>,
    armed: bool,
}

impl UmountRegistrationGuard {
    pub(crate) fn empty() -> Self {
        Self::default()
    }

    fn armed() -> Self {
        Self {
            registered: Vec::new(),
            armed: true,
        }
    }

    pub(crate) fn disarm(&mut self) {
        self.armed = false;
        self.registered.clear();
    }

    fn rollback(&mut self) -> Result<()> {
        if !self.armed {
            return Ok(());
        }

        let mut errors = Vec::new();
        for target in self.registered.iter().rev() {
            if let Err(error) = delete_registration(target) {
                errors.push(format!("{}: {error:#}", target.display()));
            }
        }

        if !errors.is_empty() {
            bail!(
                "failed to roll back KernelSU try-umount registrations: {}",
                errors.join(" | ")
            );
        }

        self.armed = false;
        self.registered.clear();
        Ok(())
    }
}

impl Drop for UmountRegistrationGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }

        match self.rollback() {
            Ok(()) => crate::scoped_log!(
                warn,
                "umount_mgr",
                "rolled back KernelSU try-umount registrations after transaction failure"
            ),
            Err(error) => crate::scoped_log!(
                error,
                "umount_mgr",
                "KernelSU try-umount registration rollback incomplete: error={:#}",
                error
            ),
        }
    }
}

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

pub fn commit() -> Result<UmountRegistrationGuard> {
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        Ok(UmountRegistrationGuard::empty())
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        if !crate::utils::KSU.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok(UmountRegistrationGuard::empty());
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
            return Ok(UmountRegistrationGuard::empty());
        }

        let mut guard = UmountRegistrationGuard::armed();
        let mut targets = pending.iter().cloned().collect::<Vec<_>>();
        targets.sort();

        for target in &targets {
            register_target(target).with_context(|| {
                format!(
                    "KernelSU try-umount registration failed for {}",
                    target.display()
                )
            })?;
            guard.registered.push(target.clone());
        }

        pending.clear();
        crate::scoped_log!(
            debug,
            "umount_mgr",
            "registered KernelSU try-umount targets: count={}",
            guard.registered.len()
        );
        Ok(guard)
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn register_target(target: &Path) -> Result<()> {
    let mut entry = TryUmount::new();
    entry.add(target);
    entry.flags(TryUmountFlags::MNT_DETACH);
    entry.umount()
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn scan_ksu_driver_fd() -> Option<RawFd> {
    let entries = fs::read_dir("/proc/self/fd").ok()?;
    for entry in entries.flatten() {
        let Ok(fd) = entry.file_name().to_string_lossy().parse::<RawFd>() else {
            continue;
        };
        let Ok(target) = fs::read_link(entry.path()) else {
            continue;
        };
        if target.to_string_lossy().contains("[ksu_driver]") {
            return Some(fd);
        }
    }
    None
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn open_ksu_driver_fd() -> RawFd {
    if let Some(fd) = scan_ksu_driver_fd() {
        return fd;
    }

    let mut fd = -1;
    unsafe {
        libc::syscall(
            libc::SYS_reboot,
            KSU_INSTALL_MAGIC1,
            KSU_INSTALL_MAGIC2,
            0,
            &mut fd,
        );
    }
    fd
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn ksu_driver_fd() -> Result<RawFd> {
    let fd = *KSU_DRIVER_FD.get_or_init(open_ksu_driver_fd);
    if fd < 0 {
        bail!("KernelSU driver file descriptor is unavailable");
    }
    Ok(fd)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn delete_registration(target: &Path) -> Result<()> {
    let path = CString::new(target.as_os_str().as_bytes())
        .with_context(|| format!("try-umount target contains NUL: {}", target.display()))?;
    let cmd = AddTryUmountCmd::delete(path.as_ptr() as u64);
    let ret = unsafe { libc::ioctl(ksu_driver_fd()?, KSU_IOCTL_ADD_TRY_UMOUNT, &cmd) };
    if ret < 0 {
        return Err(std::io::Error::last_os_error()).with_context(|| {
            format!(
                "failed to delete KernelSU try-umount registration {}",
                target.display()
            )
        });
    }
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn delete_registration(_target: &Path) -> Result<()> {
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

    #[test]
    fn delete_command_uses_the_single_entry_delete_mode() {
        let cmd = AddTryUmountCmd::delete(0x1234);

        assert_eq!(cmd.arg, 0x1234);
        assert_eq!(cmd.flags, 0);
        assert_eq!(cmd.mode, 2);
    }
}

#[cfg(all(test, target_os = "linux"))]
mod privileged_validation {
    use std::{
        collections::HashMap,
        ffi::CStr,
        fs,
        path::{Path, PathBuf},
        sync::atomic::Ordering,
    };

    use anyhow::{Context, Result, bail};
    use procfs::process::Process;
    use rustix::mount::{MountFlags, UnmountFlags, mount, unmount};

    use super::PENDING;
    use crate::{
        core::inventory::Module,
        domain::{ModuleRules, MountMode},
        mount::magic_mount::{MagicMountOptions, magic_mount},
    };

    const PROBE_PATH: &str = "/system/etc/rehybird-upstream-sync.conf";

    fn reset_pending(ksu_enabled: bool) -> Result<()> {
        crate::utils::KSU.store(ksu_enabled, Ordering::Relaxed);
        PENDING
            .lock()
            .map_err(|_| anyhow::anyhow!("pending umount lock was poisoned"))?
            .clear();
        Ok(())
    }

    fn pending_targets() -> Result<Vec<PathBuf>> {
        let mut targets = PENDING
            .lock()
            .map_err(|_| anyhow::anyhow!("pending umount lock was poisoned"))?
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        targets.sort();
        Ok(targets)
    }

    fn detach_if_mounted(path: &Path) {
        let _ = unmount(path, UnmountFlags::DETACH);
    }

    fn cleanup_namespace() {
        detach_if_mounted(Path::new(PROBE_PATH));
        detach_if_mounted(Path::new("/system/etc"));
        detach_if_mounted(Path::new("/system"));
        let _ = fs::remove_dir_all("/system");
        let _ = reset_pending(false);
    }

    fn run_privileged_validation() -> Result<()> {
        if unsafe { libc::geteuid() } != 0 {
            bail!("privileged mount validation must run as root inside a private mount namespace");
        }

        fs::create_dir_all("/system")?;
        mount(
            "rehybird-validation-system",
            "/system",
            "tmpfs",
            MountFlags::empty(),
            None::<&CStr>,
        )
        .context("failed to mount private /system tmpfs")?;
        fs::create_dir_all("/system/etc")?;
        crate::sys::fs::lsetfilecon("/system/etc", "u:object_r:system_file:s0")?;

        let modules = tempfile::tempdir()?;
        let module_path = modules.path().join("rehybird_validation");
        let source_file = module_path.join("system/etc/rehybird-upstream-sync.conf");
        fs::create_dir_all(source_file.parent().context("probe source has no parent")?)?;
        fs::write(module_path.join("module.prop"), "id=rehybird_validation\n")?;
        fs::write(&source_file, "upstream-sync-ok\n")?;

        let workspace = tempfile::Builder::new()
            .prefix("rehybird_magic_validation_")
            .tempdir_in("/mnt")?;
        let module = Module {
            id: "rehybird_validation".to_string(),
            source_path: module_path,
            rules: ModuleRules {
                default_mode: MountMode::Magic,
                paths: HashMap::new(),
            },
        };
        let managed_partitions = vec!["system".to_string()];

        reset_pending(true)?;
        let (mounted_ids, _) = magic_mount(
            workspace.path(),
            modules.path(),
            MagicMountOptions {
                mount_source: "rehybird-validation-workspace",
                managed_partitions: &managed_partitions,
            },
            &[module],
            true,
        )?;

        if mounted_ids != vec!["rehybird_validation".to_string()] {
            bail!("unexpected mounted module set: {mounted_ids:?}");
        }
        let probe = fs::read_to_string(PROBE_PATH)?;
        if probe != "upstream-sync-ok\n" {
            bail!("mounted probe content mismatch: {probe:?}");
        }

        let pending = pending_targets()?;
        if pending.iter().any(|path| path.starts_with("/mnt")) {
            bail!("temporary /mnt target leaked into KernelSU registration set: {pending:?}");
        }
        if pending != vec![PathBuf::from("/system/etc")] {
            bail!("unexpected final KernelSU registration set: {pending:?}");
        }

        let workspace_mounts = Process::myself()?
            .mountinfo()?
            .into_iter()
            .filter(|entry| entry.mount_point.starts_with(workspace.path()))
            .map(|entry| entry.mount_point)
            .collect::<Vec<_>>();
        if !workspace_mounts.is_empty() {
            bail!("temporary Magic Mount workspace remained mounted: {workspace_mounts:?}");
        }

        Ok(())
    }

    #[test]
    #[ignore = "requires root and a private Linux mount namespace"]
    fn privileged_magic_mount_excludes_temporary_worktree_from_unmount_registration() {
        let result = run_privileged_validation();
        cleanup_namespace();
        result.unwrap();
    }
}
