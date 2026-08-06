// Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-only

mod custom_bind;
mod magic;
mod overlay;

use std::{collections::BTreeSet, path::Path};

use anyhow::{Context, Result, bail};

#[cfg(feature = "kasumi")]
use crate::core::kasumi_coordinator::KasumiCoordinator;
#[cfg(any(target_os = "linux", target_os = "android"))]
use crate::mount::umount_mgr;
use crate::{
    conf::config,
    core::{
        failure::{FailureStage, ModuleStageFailure},
        inventory::Module,
        ops::plan::{MountPlan, OverlayOperation},
        runtime_state::MountStatistics,
    },
    utils,
};

pub struct ExecutionResult {
    pub overlay_module_ids: Vec<String>,
    pub overlay_partitions: Vec<String>,
    pub magic_module_ids: Vec<String>,
    pub custom_mount_targets: Vec<String>,
    #[cfg(feature = "kasumi")]
    pub kasumi_module_ids: Vec<String>,
    pub kasumi_runtime_enabled: bool,
    pub mount_stats: MountStatistics,
}

impl ExecutionResult {
    pub fn kasumi_count(&self) -> usize {
        #[cfg(feature = "kasumi")]
        {
            self.kasumi_module_ids.len()
        }
        #[cfg(not(feature = "kasumi"))]
        {
            0
        }
    }
}

#[cfg(feature = "kasumi")]
struct KasumiRuntimeGuard {
    armed: bool,
}

#[cfg(feature = "kasumi")]
impl KasumiRuntimeGuard {
    fn new(armed: bool) -> Self {
        Self { armed }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

#[cfg(feature = "kasumi")]
impl Drop for KasumiRuntimeGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }

        fn record(errors: &mut Vec<String>, operation: &str, result: Result<()>) {
            if let Err(error) = result {
                errors.push(format!("{operation}: {error:#}"));
            }
        }

        let mut errors = Vec::new();
        record(
            &mut errors,
            "disable runtime",
            crate::sys::kasumi::set_enabled(false),
        );
        record(
            &mut errors,
            "clear mount rules",
            crate::sys::kasumi::clear_rules(),
        );
        record(
            &mut errors,
            "clear maps rules",
            crate::sys::kasumi::clear_maps_rules(),
        );
        record(
            &mut errors,
            "disable debug",
            crate::sys::kasumi::set_debug(false),
        );
        record(
            &mut errors,
            "disable stealth",
            crate::sys::kasumi::set_stealth(false),
        );
        record(
            &mut errors,
            "disable mount hide",
            crate::sys::kasumi::set_mount_hide(false),
        );
        record(
            &mut errors,
            "disable maps spoof",
            crate::sys::kasumi::set_maps_spoof(false),
        );
        record(
            &mut errors,
            "disable statfs spoof",
            crate::sys::kasumi::set_statfs_spoof(false),
        );
        record(
            &mut errors,
            "disable selinux fix",
            crate::sys::kasumi::set_selinux_fix(false),
        );
        record(
            &mut errors,
            "clear hidden uids",
            crate::sys::kasumi::set_hide_uids(&[]),
        );
        record(
            &mut errors,
            "clear cmdline spoof",
            crate::sys::kasumi::set_cmdline_str(""),
        );
        let empty_uname = crate::sys::kasumi::KasumiSpoofUname::default();
        record(
            &mut errors,
            "clear scoped uname spoof",
            crate::sys::kasumi::set_uname(&empty_uname),
        );
        record(
            &mut errors,
            "clear global uname spoof",
            crate::sys::kasumi::restore_uname_global(),
        );

        if errors.is_empty() {
            crate::scoped_log!(warn, "executor", "Kasumi runtime rolled back after failure");
        } else {
            crate::scoped_log!(
                error,
                "executor",
                "Kasumi rollback incomplete: failures={}",
                errors.join(" | ")
            );
        }
    }
}

pub struct Executor;

impl Executor {
    pub fn execute<P>(
        plan: &mut MountPlan,
        modules: &[Module],
        config: &config::Config,
        tempdir: P,
    ) -> Result<ExecutionResult>
    where
        P: AsRef<Path>,
    {
        crate::scoped_log!(
            info,
            "executor",
            "start: overlay_ops={}, preselected_magic_modules={}, preselected_kasumi_modules={}, kasumi_fallback_modules={}",
            plan.overlay_ops.len(),
            plan.magic_module_ids.len(),
            plan.kasumi_count(),
            plan.kasumi_fallback_ids().len()
        );
        let mut final_magic_ids: BTreeSet<String> = plan.magic_module_ids.iter().cloned().collect();
        let mut final_overlay_ids: BTreeSet<String> = BTreeSet::new();
        let mut final_overlay_partitions: BTreeSet<String> = BTreeSet::new();
        #[cfg(feature = "kasumi")]
        let planned_kasumi_ids = plan.kasumi_module_ids.clone();
        let mut mount_stats = MountStatistics::default();
        #[cfg(feature = "kasumi")]
        let kasumi = KasumiCoordinator::new(config);

        #[cfg(feature = "kasumi")]
        let kasumi_available = if config.kasumi.enabled {
            kasumi.reset_runtime().map_err(|err| {
                ModuleStageFailure::new(
                    FailureStage::Execute,
                    planned_kasumi_ids.clone(),
                    anyhow::anyhow!("Failed to reset Kasumi runtime: {:#}", err),
                )
            })?
        } else {
            crate::scoped_log!(
                debug,
                "executor",
                "kasumi disabled: skip_runtime_reset=true"
            );
            false
        };
        #[cfg(feature = "kasumi")]
        if !kasumi_available && !planned_kasumi_ids.is_empty() {
            return Err(ModuleStageFailure::new(
                FailureStage::Execute,
                planned_kasumi_ids.clone(),
                anyhow::anyhow!("Kasumi became unavailable before execution"),
            )
            .into());
        }

        #[cfg(feature = "kasumi")]
        let mut kasumi_guard = KasumiRuntimeGuard::new(kasumi_available);
        #[cfg(feature = "kasumi")]
        let final_kasumi_ids = plan.kasumi_module_ids.clone();
        #[cfg(feature = "kasumi")]
        let kasumi_runtime_enabled = if config.kasumi.enabled {
            kasumi.apply_runtime(plan, modules).map_err(|error| {
                ModuleStageFailure::new(
                    FailureStage::Execute,
                    final_kasumi_ids.clone(),
                    anyhow::anyhow!(
                        "Failed to apply Kasumi before other mount backends: {:#}",
                        error
                    ),
                )
            })?
        } else {
            crate::scoped_log!(
                debug,
                "executor",
                "kasumi disabled: skip_runtime_apply=true"
            );
            false
        };
        #[cfg(not(feature = "kasumi"))]
        let kasumi_runtime_enabled = false;

        if Self::is_supported()? {
            crate::scoped_log!(info, "executor", "overlayfs: supported=true");
            for op in &plan.overlay_ops {
                crate::scoped_log!(
                    info,
                    "executor",
                    "overlay apply: partition={}, target={}, layers={}",
                    op.partition_name,
                    op.target,
                    op.lowerdirs.len()
                );

                #[cfg(feature = "kasumi")]
                let overlay_result = overlay::mount_overlay(op, config, &kasumi);
                #[cfg(not(feature = "kasumi"))]
                let overlay_result = overlay::mount_overlay(op, config);

                match overlay_result {
                    Ok(ids) => {
                        crate::scoped_log!(
                            info,
                            "executor",
                            "overlay success: target={}, modules={}",
                            op.target,
                            ids.len()
                        );
                        final_overlay_partitions.insert(op.partition_name.clone());
                        final_overlay_ids.extend(ids);
                        mount_stats.record_overlay_mount();
                    }
                    Err(err) => {
                        let involved_modules = collect_involved_modules(op);
                        if is_symlink_loop_error(&err) {
                            crate::scoped_log!(
                                error,
                                "executor",
                                "overlay failed: target={}, reason=symlink_loop",
                                op.target
                            );
                        } else {
                            crate::scoped_log!(
                                error,
                                "executor",
                                "overlay failed: target={}, reason=non_symlink_loop",
                                op.target
                            );
                        }
                        return Err(ModuleStageFailure::new(
                            FailureStage::Execute,
                            involved_modules,
                            anyhow::anyhow!("Overlay mount failed for {}: {:#}", op.target, err),
                        )
                        .into());
                    }
                }
            }
        } else {
            if !plan.overlay_ops.is_empty() {
                bail!("[executor] overlayfs unsupported and overlay operations are pending");
            }
            crate::scoped_log!(
                info,
                "executor",
                "overlayfs: supported=false, pending_overlay_ops=0"
            );
        }

        let magic_need_list: Vec<String> = final_magic_ids.iter().cloned().collect();

        if !magic_need_list.is_empty() {
            crate::scoped_log!(
                info,
                "executor",
                "magic apply: modules={}, kasumi_fallback_modules={}",
                magic_need_list.join(", "),
                plan.kasumi_fallback_ids().len()
            );
            let (mounted_ids, magic_stats) = magic::mount_magic(
                modules,
                &magic_need_list,
                plan.kasumi_fallback_ids(),
                config,
                tempdir.as_ref(),
            )
            .map_err(|err| {
                ModuleStageFailure::new(
                    FailureStage::Execute,
                    magic_need_list.clone(),
                    anyhow::anyhow!(
                        "Failed to mount Magic Mount modules [{}]: {:#}",
                        magic_need_list.join(", "),
                        err
                    ),
                )
            })?;
            mount_stats.merge(&magic_stats);
            let mounted_ids: BTreeSet<String> = mounted_ids.into_iter().collect();
            final_magic_ids.retain(|id| mounted_ids.contains(id));
            crate::scoped_log!(
                info,
                "executor",
                "magic complete: mounted_modules={}",
                mounted_ids.len()
            );
        }

        let (custom_mount_targets, custom_stats) = custom_bind::mount_custom_binds(config)
            .context("Failed to apply custom bind mounts")?;
        mount_stats.merge(&custom_stats);

        #[cfg(feature = "kasumi")]
        if kasumi_runtime_enabled && let Err(error) = crate::sys::kasumi::fix_mounts() {
            crate::scoped_log!(
                warn,
                "executor",
                "Kasumi mount-id refresh failed after other backends: error={:#}",
                error
            );
        }

        #[cfg(any(target_os = "linux", target_os = "android"))]
        if !config.disable_umount {
            umount_mgr::commit().context("Failed to commit umountable mount list")?;
        }

        let result_overlay: Vec<String> = final_overlay_ids.into_iter().collect();
        let result_magic: Vec<String> = final_magic_ids.into_iter().collect();
        #[cfg(not(feature = "kasumi"))]
        let kasumi_count = 0usize;
        #[cfg(feature = "kasumi")]
        let kasumi_count = final_kasumi_ids.len();

        crate::scoped_log!(
            info,
            "executor",
            "complete: overlay_modules={}, magic_modules={}, custom_mounts={}, kasumi_modules={}",
            result_overlay.len(),
            result_magic.len(),
            custom_mount_targets.len(),
            kasumi_count
        );

        #[cfg(feature = "kasumi")]
        kasumi_guard.disarm();

        Ok(ExecutionResult {
            overlay_module_ids: result_overlay,
            overlay_partitions: final_overlay_partitions.into_iter().collect(),
            magic_module_ids: result_magic,
            custom_mount_targets,
            #[cfg(feature = "kasumi")]
            kasumi_module_ids: final_kasumi_ids,
            kasumi_runtime_enabled,
            mount_stats,
        })
    }

    fn is_supported() -> Result<bool> {
        crate::mount::overlayfs::utils::is_overlay_supported()
    }
}

fn is_symlink_loop_error(err: &anyhow::Error) -> bool {
    let mut cursor = Some(err.as_ref() as &(dyn std::error::Error + 'static));
    while let Some(current) = cursor {
        let msg = current.to_string();
        if msg.contains("Too many symbolic links") || msg.contains("os error 40") {
            return true;
        }
        cursor = current.source();
    }
    false
}

fn collect_involved_modules(op: &OverlayOperation) -> Vec<String> {
    let mut involved_modules: Vec<String> = op
        .lowerdirs
        .iter()
        .filter_map(|p| utils::extract_module_id(p))
        .collect();
    involved_modules.sort();
    involved_modules.dedup();
    involved_modules
}
