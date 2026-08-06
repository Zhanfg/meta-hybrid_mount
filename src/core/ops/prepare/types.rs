// Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-only

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Component, Path, PathBuf},
};

use crate::{core::ops::plan::PrepareMetrics, domain::MountMode};

pub(super) const SHALLOW_OVERLAY_DIR: &str = ".hybrid_overlay";

#[derive(Debug, Default)]
pub(super) struct ModulePlanOutcome {
    pub(super) overlay_groups: BTreeMap<PathBuf, (String, Vec<PathBuf>)>,
    pub(super) magic: bool,
    pub(super) kasumi: bool,
}

impl ModulePlanOutcome {
    pub(super) fn has_mount_result(&self) -> bool {
        !self.overlay_groups.is_empty() || self.magic || self.kasumi
    }
}

#[derive(Debug, Default)]
pub(super) struct ModulePrepareOutcome {
    pub(super) has_mount_content: bool,
    pub(super) opaque_dirs: Vec<PathBuf>,
    pub(super) plan: ModulePlanOutcome,
}

pub(super) struct ProcessingItem {
    pub(super) source_dir: PathBuf,
    pub(super) copy_dir: PathBuf,
    pub(super) final_dir: PathBuf,
    pub(super) shallow_copy_dir: PathBuf,
    pub(super) shallow_final_dir: PathBuf,
    pub(super) system_target: PathBuf,
    pub(super) relative_path: PathBuf,
    pub(super) partition_label: String,
    pub(super) plan_active: bool,
    pub(super) materialize_tree: bool,
    pub(super) count_mount_content: bool,
}

pub(super) struct EntryState {
    pub(super) direct_non_dir_entries: bool,
    pub(super) has_child_dirs: bool,
    pub(super) has_replace_marker: bool,
}

pub(super) struct ModeDecision {
    pub(super) requested_mode: MountMode,
    pub(super) effective_mode: MountMode,
    pub(super) has_descendant_rules: bool,
}

pub(super) struct PrepareContext {
    pub(super) managed_partitions: HashSet<String>,
    pub(super) system_root: PathBuf,
    pub(super) target_cache: HashMap<PathBuf, PathBuf>,
    pub(super) metrics: PrepareMetrics,
}

impl PrepareContext {
    pub(super) fn new(managed_partitions: HashSet<String>, system_root: PathBuf) -> Self {
        Self {
            managed_partitions,
            system_root,
            target_cache: HashMap::new(),
            metrics: PrepareMetrics::default(),
        }
    }

    pub(super) fn resolved_target_is_managed(&self, target: &Path) -> bool {
        let Ok(relative) = target.strip_prefix(&self.system_root) else {
            return false;
        };
        let Some(Component::Normal(partition)) = relative.components().next() else {
            return false;
        };
        partition
            .to_str()
            .is_some_and(|partition| self.managed_partitions.contains(partition))
    }

    pub(super) fn record_copy(&mut self, bytes: u64) {
        self.metrics.copied_entries += 1;
        self.metrics.copied_bytes = self.metrics.copied_bytes.saturating_add(bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolved_targets_must_stay_under_managed_partitions() {
        let context = PrepareContext::new(
            HashSet::from(["system".to_string(), "vendor".to_string()]),
            PathBuf::from("/"),
        );

        assert!(context.resolved_target_is_managed(Path::new("/system/etc")));
        assert!(context.resolved_target_is_managed(Path::new("/vendor/lib64")));
        assert!(!context.resolved_target_is_managed(Path::new("/data/local/tmp")));
        assert!(!context.resolved_target_is_managed(Path::new("/")));
    }

    #[test]
    fn resolved_targets_cannot_escape_a_test_system_root() {
        let context = PrepareContext::new(
            HashSet::from(["system".to_string()]),
            PathBuf::from("/tmp/sysroot"),
        );

        assert!(context.resolved_target_is_managed(Path::new("/tmp/sysroot/system/bin")));
        assert!(!context.resolved_target_is_managed(Path::new("/tmp/outside/system/bin")));
        assert!(!context.resolved_target_is_managed(Path::new("/tmp/sysroot/data")));
    }
}
