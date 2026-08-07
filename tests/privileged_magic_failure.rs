// Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-only

#![cfg(target_os = "linux")]

use std::{collections::HashMap, ffi::CStr, fs, path::Path};

use anyhow::{Context, Result, bail};
use hybrid_mount::{
    core::inventory::Module,
    domain::{ModuleRules, MountMode},
    mount::magic_mount::{MagicMountOptions, magic_mount},
};
use procfs::process::Process;
use rustix::mount::{MountFlags, UnmountFlags, mount, unmount};

const PROBE_PATH: &str = "/system/etc/rehybird-failure-probe.conf";

fn detach_if_mounted(path: &Path) {
    let _ = unmount(path, UnmountFlags::DETACH);
}

fn cleanup_namespace() {
    detach_if_mounted(Path::new(PROBE_PATH));
    detach_if_mounted(Path::new("/system/etc"));
    detach_if_mounted(Path::new("/system"));
    let _ = fs::remove_dir_all("/system");
}

fn run_failure_cleanup_validation() -> Result<()> {
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

    // Deliberately omit the test SELinux xattr. The real Magic Mount code will
    // mount its temporary workspace first, then fail while cloning metadata for
    // /system/etc. This exercises the post-mount failure cleanup path.
    fs::create_dir_all("/system/etc")?;

    let modules = tempfile::tempdir()?;
    let module_path = modules.path().join("rehybird_failure_validation");
    let source_file = module_path.join("system/etc/rehybird-failure-probe.conf");
    fs::create_dir_all(source_file.parent().context("probe source has no parent")?)?;
    fs::write(
        module_path.join("module.prop"),
        "id=rehybird_failure_validation\n",
    )?;
    fs::write(&source_file, "must-not-be-mounted\n")?;

    let workspace = tempfile::Builder::new()
        .prefix("rehybird_magic_failure_")
        .tempdir_in("/mnt")?;
    let module = Module {
        id: "rehybird_failure_validation".to_string(),
        source_path: module_path,
        rules: ModuleRules {
            default_mode: MountMode::Magic,
            paths: HashMap::new(),
        },
    };
    let managed_partitions = vec!["system".to_string()];

    let error = magic_mount(
        workspace.path(),
        modules.path(),
        MagicMountOptions {
            mount_source: "rehybird-validation-workspace",
            managed_partitions: &managed_partitions,
        },
        &[module],
        false,
    )
    .expect_err("metadata failure was expected");

    let error_text = format!("{error:#}");
    if !error_text.contains("Failed to get SELinux context") {
        bail!("unexpected injected failure: {error_text}");
    }
    if Path::new(PROBE_PATH).exists() {
        bail!("probe unexpectedly appeared at the final target");
    }

    let workspace_mounts = Process::myself()?
        .mountinfo()?
        .into_iter()
        .filter(|entry| entry.mount_point.starts_with(workspace.path()))
        .map(|entry| entry.mount_point)
        .collect::<Vec<_>>();
    if !workspace_mounts.is_empty() {
        bail!("failed Magic Mount left workspace mounts behind: {workspace_mounts:?}");
    }
    if workspace.path().join("workdir").exists() {
        bail!("failed Magic Mount left its workdir behind");
    }

    Ok(())
}

#[test]
#[ignore = "requires root and a private Linux mount namespace"]
fn privileged_magic_mount_cleans_workspace_after_metadata_failure() {
    let result = run_failure_cleanup_validation();
    cleanup_namespace();
    result.unwrap();
}
