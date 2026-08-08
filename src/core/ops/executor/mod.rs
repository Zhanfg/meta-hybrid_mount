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
use crate::{
    conf::config,
    core::{
        failure::{FailureStage, ModuleStageFailure},
        inventory::Module,
        ops::plan::{MountPlan, OverlayOperation},
        runtime_state::MountStatistics,
    },
    mount::{umount_mgr, vfs},
    utils,
};

pub struct ExecutionResult {
    pub vfs_module_ids: Vec<String>,
    pub vfs_partitions: Vec<String>,
    pub vfs_backend: Option<String>,
    pub vfs_fallback_module_ids: Vec<String>,
    pub overlay_module_ids: Vec<String>,
    pub overlay_partitions: Vec<String>,
    pub magic_module_ids: Vec<String>,
    pub custom_mount_targets: Vec<String>,
    #[cfg(feature = "kasumi")]
    pub kasumi_module_ids: Vec<String>,
    pub kasumi_runtime_enabled: bool,
    pub mount_stats: MountStatistics,
    umount_guard: umount_mgr::UmountRegistrationGuard,
    #[cfg(feature = "kasumi")]
    kasumi_guard: KasumiRuntimeGuard,
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

    pub(crate) fn commit_runtime(&mut self) {
        self.umount_guard.disarm();
        #[cfg(feature = "kasumi")]
        self.kasumi_guard.disarm();
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
        match crate::mount::kasumi::rollback_runtime() {
            Ok(()) => crate::scoped_log!(
                warn,
                "executor",
                "Kasumi runtime rolled back after transaction failure"
            ),
            Err(error) => crate::scoped_log!(
                error,
                "executor",
                "Kasumi rollback incomplete: error={:#}",
                error
            ),
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
            "start: vfs_ops={}, overlay_ops={}, preselected_magic_modules={}, preselected_kasumi_modules={}",
            plan.vfs_ops.len(),
            plan.overlay_ops.len(),
            plan.magic_module_ids.len(),
            plan.kasumi_count(),
        );
        let mut final_magic_ids: BTreeSet<String> = plan.magic_module_ids.iter().cloned().collect();
        let mut final_vfs_ids: BTreeSet<String> = plan.vfs_module_ids.iter().cloned().collect();
        let mut final_vfs_partitions = BTreeSet::new();
        let mut final_vfs_backend = plan.vfs_ops.first().map(|op| op.backend.clone());
        let mut vfs_fallback_ids: BTreeSet<String> =
            plan.vfs_fallback_module_ids.iter().cloned().collect();
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
        let kasumi_guard = KasumiRuntimeGuard::new(kasumi_available);
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
            false
        };
        #[cfg(not(feature = "kasumi"))]
        let kasumi_runtime_enabled = false;

        if !plan.vfs_ops.is_empty() {
            let mut mounted_targets = Vec::new();
            let mut runtime_failure = None;
            for op in &plan.vfs_ops {
                crate::scoped_log!(
                    info,
                    "executor:vfs",
                    "apply: backend={}, partition={}, target={}, module_branches={}",
                    op.backend,
                    op.partition_name,
                    op.target.display(),
                    op.lowerdirs.len()
                );
                match vfs::mount_union(op) {
                    Ok(()) => mounted_targets.push(op.target.clone()),
                    Err(error) => {
                        runtime_failure = Some(anyhow::anyhow!(
                            "VFS mount failed for {} using {}: {:#}",
                            op.target.display(),
                            op.backend,
                            error
                        ));
                        break;
                    }
                }
            }

            if let Some(error) = runtime_failure {
                vfs::rollback_targets(&mounted_targets).with_context(|| {
                    format!("failed to roll back partial VFS transaction after: {error:#}")
                })?;
                final_magic_ids.extend(final_vfs_ids.iter().cloned());
                vfs_fallback_ids.extend(final_vfs_ids.iter().cloned());
                final_vfs_ids.clear();
                final_vfs_backend = None;
                crate::scoped_log!(
                    warn,
                    "executor:vfs",
                    "runtime fallback to Magic: modules={}, error={:#}",
                    vfs_fallback_ids
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(","),
                    error
                );
            } else {
                for op in &plan.vfs_ops {
                    umount_mgr::send_umountable(&op.target).with_context(|| {
                        format!("failed to register VFS target {}", op.target.display())
                    })?;
                    final_vfs_partitions.insert(op.partition_name.clone());
                    mount_stats.record_vfs_mount();
                }
            }
        }

        if Self::is_supported()? {
            for op in &plan.overlay_ops {
                #[cfg(feature = "kasumi")]
                let overlay_result = overlay::mount_overlay(op, config, &kasumi);
                #[cfg(not(feature = "kasumi"))]
                let overlay_result = overlay::mount_overlay(op, config);

                match overlay_result {
                    Ok(ids) => {
                        final_overlay_partitions.insert(op.partition_name.clone());
                        final_overlay_ids.extend(ids);
                        mount_stats.record_overlay_mount();
                    }
                    Err(err) => {
                        let involved_modules = collect_involved_modules(op);
                        return Err(ModuleStageFailure::new(
                            FailureStage::Execute,
                            involved_modules,
                            anyhow::anyhow!("Overlay mount failed for {}: {:#}", op.target, err),
                        )
                        .into());
                    }
                }
            }
        } else if !plan.overlay_ops.is_empty() {
            bail!("[executor] overlayfs unsupported and overlay operations are pending");
        }

        let magic_need_list: Vec<String> = final_magic_ids.iter().cloned().collect();
        let forced_magic: Vec<String> = vfs_fallback_ids.iter().cloned().collect();
        if !magic_need_list.is_empty() {
            let (mounted_ids, magic_stats) = magic::mount_magic(
                modules,
                &magic_need_list,
                plan.kasumi_fallback_ids(),
                &forced_magic,
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
        }

        let (custom_mount_targets, custom_stats) = custom_bind::mount_custom_binds(config)
            .context("Failed to apply custom bind mounts")?;
        mount_stats.merge(&custom_stats);

        #[cfg(feature = "kasumi")]
        if kasumi_runtime_enabled && let Err(error) = crate::sys::kasumi::fix_mounts() {
            crate::scoped_log!(
                warn,
                "executor",
                "Kasumi mount-id refresh failed: error={:#}",
                error
            );
        }

        let umount_guard = if config.disable_umount {
            umount_mgr::UmountRegistrationGuard::empty()
        } else {
            umount_mgr::commit().context("Failed to commit umountable mount list")?
        };

        let result_vfs: Vec<String> = final_vfs_ids.into_iter().collect();
        let result_overlay: Vec<String> = final_overlay_ids.into_iter().collect();
        let result_magic: Vec<String> = final_magic_ids.into_iter().collect();

        crate::scoped_log!(
            info,
            "executor",
            "complete: vfs_modules={}, vfs_partitions={}, vfs_fallback_modules={}, overlay_modules={}, magic_modules={}, custom_mounts={}, kasumi_modules={}",
            result_vfs.len(),
            final_vfs_partitions.len(),
            vfs_fallback_ids.len(),
            result_overlay.len(),
            result_magic.len(),
            custom_mount_targets.len(),
            {
                #[cfg(feature = "kasumi")]
                {
                    final_kasumi_ids.len()
                }
                #[cfg(not(feature = "kasumi"))]
                {
                    0usize
                }
            }
        );

        Ok(ExecutionResult {
            vfs_module_ids: result_vfs,
            vfs_partitions: final_vfs_partitions.into_iter().collect(),
            vfs_backend: final_vfs_backend,
            vfs_fallback_module_ids: vfs_fallback_ids.into_iter().collect(),
            overlay_module_ids: result_overlay,
            overlay_partitions: final_overlay_partitions.into_iter().collect(),
            magic_module_ids: result_magic,
            custom_mount_targets,
            #[cfg(feature = "kasumi")]
            kasumi_module_ids: final_kasumi_ids,
            kasumi_runtime_enabled,
            mount_stats,
            umount_guard,
            #[cfg(feature = "kasumi")]
            kasumi_guard,
        })
    }

    fn is_supported() -> Result<bool> {
        crate::mount::overlayfs::utils::is_overlay_supported()
    }
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
