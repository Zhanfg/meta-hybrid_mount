from pathlib import Path


def replace_once(path: Path, old: str, new: str) -> None:
    text = path.read_text()
    count = text.count(old)
    if count != 1:
        raise SystemExit(
            f"{path}: expected one match, found {count}: {old[:140]!r}"
        )
    path.write_text(text.replace(old, new, 1))


overlay = Path("src/core/ops/executor/overlay.rs")
replace_once(
    overlay,
    '''    mount_overlay_base(op, config)?;
    if let Some(kasumi) = kasumi {
        kasumi.hide_overlay_xattrs(Path::new(&op.target))?;
    }
    Ok(super::collect_involved_modules(op))
''',
    '''    mount_overlay_base(op, config)?;
    if let Some(kasumi) = kasumi
        && let Err(error) = kasumi.hide_overlay_xattrs(Path::new(&op.target))
    {
        crate::scoped_log!(
            warn,
            "executor:overlay",
            "overlay mounted but Kasumi xattr hiding failed: target={}, error={:#}",
            op.target,
            error
        );
    }
    Ok(super::collect_involved_modules(op))
''',
)
replace_once(
    overlay,
    '''    #[cfg(any(target_os = "linux", target_os = "android"))]
    if !config.disable_umount {
        umount_mgr::send_umountable(&op.target)?;
    }

    Ok(())
''',
    '''    #[cfg(any(target_os = "linux", target_os = "android"))]
    if !config.disable_umount
        && let Err(error) = umount_mgr::send_umountable(&op.target)
    {
        crate::scoped_log!(
            warn,
            "executor:overlay",
            "overlay mounted but umount registration failed: target={}, error={:#}",
            op.target,
            error
        );
    }

    Ok(())
''',
)

magic = Path("src/mount/magic_mount/mod.rs")
replace_once(
    magic,
    '''        #[cfg(any(target_os = "linux", target_os = "android"))]
        if self.umount {
            send_umountable(target)?;
        }

        remount_readonly(target, target)?;
        context.stats.record_file();
''',
    '''        if let Err(error) = remount_readonly(target, target) {
            let cleanup_error = unmount(target, UnmountFlags::DETACH).err();
            if let Some(cleanup_error) = cleanup_error {
                bail!(
                    "failed to make bind mount readonly and rollback failed for {}: remount={:#}; rollback={:#}",
                    target.display(),
                    error,
                    cleanup_error
                );
            }
            return Err(error);
        }

        #[cfg(any(target_os = "linux", target_os = "android"))]
        if self.umount && let Err(error) = send_umountable(target) {
            crate::scoped_log!(
                warn,
                "magic",
                "file mounted but umount registration failed: target={}, error={:#}",
                target.display(),
                error
            );
        }

        context.stats.record_file();
''',
)
replace_once(
    magic,
    '''            #[cfg(any(target_os = "linux", target_os = "android"))]
            if self.umount {
                send_umountable(&self.path)?;
            }
            context.stats.record_dir();
''',
    '''            #[cfg(any(target_os = "linux", target_os = "android"))]
            if self.umount && let Err(error) = send_umountable(&self.path) {
                crate::scoped_log!(
                    warn,
                    "magic",
                    "directory mounted but umount registration failed: target={}, error={:#}",
                    self.path.display(),
                    error
                );
            }
            context.stats.record_dir();
''',
)
replace_once(
    magic,
    '''        ret?;
        cleanup_result?;
        Ok((mounted_module_ids, context.stats))
''',
    '''        if let Err(error) = cleanup_result {
            crate::scoped_log!(
                warn,
                "magic",
                "temporary workspace cleanup failed after mount execution: path={}, error={:#}",
                tmp_dir.display(),
                error
            );
        }
        ret?;
        Ok((mounted_module_ids, context.stats))
''',
)

custom = Path("src/mount/custom_bind.rs")
replace_once(
    custom,
    '''#[cfg(any(target_os = "linux", target_os = "android"))]
use rustix::mount::{MountFlags, mount_bind, mount_remount};
''',
    '''#[cfg(any(target_os = "linux", target_os = "android"))]
use rustix::mount::{MountFlags, UnmountFlags, mount_bind, mount_remount, unmount};
''',
)
replace_once(
    custom,
    '''    for mount in mounts {
        let applied = apply_one(mount, disable_umount).with_context(|| {
            format!(
                "failed to apply custom bind {} -> {}",
                mount.source.display(),
                mount.target.display()
            )
        })?;
        crate::scoped_log!(
            info,
            "custom_bind",
            "mounted: source={}, target={}",
            applied.source.display(),
            applied.target.display()
        );
        applied_mounts.push(applied);
    }
''',
    '''    for mount in mounts {
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
''',
)
replace_once(
    custom,
    '''#[cfg(any(target_os = "linux", target_os = "android"))]
fn bind_mount_checked(source: &Path, target: &Path, disable_umount: bool) -> Result<()> {
    mount_bind(source, target).with_context(|| {
        format!(
            "failed to bind mount {} to {}",
            source.display(),
            target.display()
        )
    })?;

    mount_remount(target, MountFlags::RDONLY | MountFlags::BIND, "").with_context(|| {
        format!(
            "failed to remount custom bind readonly: {}",
            target.display()
        )
    })?;

    if !disable_umount {
        crate::mount::umount_mgr::send_umountable(target).with_context(|| {
            format!(
                "failed to register custom bind target as umountable: {}",
                target.display()
            )
        })?;
    }

    Ok(())
}
''',
    '''#[cfg(any(target_os = "linux", target_os = "android"))]
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

    if !disable_umount
        && let Err(error) = crate::mount::umount_mgr::send_umountable(target)
    {
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
''',
)

finalization = Path("src/core/runtime_finalization.rs")
replace_once(
    finalization,
    '''    let state =
        RuntimeState::build_from_execution(config, storage_mode, mount_point, result, inventory)?;
    state.save()?;

    crate::scoped_log!(
        info,
        "runtime_finalization",
        "complete: active_mounts={}, skip_mount_modules={}",
        state.active_mounts.len(),
        state.skip_mount_modules.len()
    );

    Ok(())
''',
    '''    let state = match RuntimeState::build_from_execution(
        config,
        storage_mode,
        mount_point,
        result,
        inventory,
    ) {
        Ok(state) => state,
        Err(error) => {
            crate::scoped_log!(
                warn,
                "runtime_finalization",
                "mounts are active but runtime state collection failed: error={:#}",
                error
            );
            return Ok(());
        }
    };

    if let Err(error) = state.save() {
        crate::scoped_log!(
            warn,
            "runtime_finalization",
            "mounts are active but runtime state persistence failed: error={:#}",
            error
        );
    }

    crate::scoped_log!(
        info,
        "runtime_finalization",
        "complete: active_mounts={}, skip_mount_modules={}",
        state.active_mounts.len(),
        state.skip_mount_modules.len()
    );

    Ok(())
''',
)

controller = Path("src/core/controller.rs")
replace_once(
    controller,
    '''        runtime_finalization::finalize(
            &self.config,
            self.state.handle.mode(),
            self.state.handle.mount_point(),
            &self.state.result,
            &self.state.inventory_summary,
        )?;

        clean_up(
            &self.tempdir,
            &self.config.kasumi.mirror_path,
            self.state.handle.mode(),
            self.config.disable_umount,
        )?;
''',
    '''        if let Err(error) = runtime_finalization::finalize(
            &self.config,
            self.state.handle.mode(),
            self.state.handle.mount_point(),
            &self.state.result,
            &self.state.inventory_summary,
        ) {
            crate::scoped_log!(
                warn,
                "controller:finalize",
                "mounts are active but runtime finalization failed: error={:#}",
                error
            );
        }

        if let Err(error) = clean_up(
            &self.tempdir,
            &self.config.kasumi.mirror_path,
            self.state.handle.mode(),
            self.config.disable_umount,
        ) {
            crate::scoped_log!(
                warn,
                "controller:finalize",
                "mounts are active but temporary cleanup failed: error={:#}",
                error
            );
        }
''',
)
