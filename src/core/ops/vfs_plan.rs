// Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-only

use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

use crate::{
    core::{backend_capabilities::BackendCapabilities, inventory::Module, ops::plan::VfsOperation},
    domain::MountMode,
};

#[derive(Debug, Default)]
pub struct VfsSelection {
    pub operations: Vec<VfsOperation>,
    pub module_ids: HashSet<String>,
    pub fallback_module_ids: Vec<String>,
}

/// Select a deliberately conservative first-generation VFS set.
///
/// A module is eligible only when it is a pure Overlay module with no path
/// overrides. VFS is therefore an optional execution backend for an existing
/// semantic mode rather than a second rules language. If another backend
/// touches the same partition, a target is missing, or the union exceeds the
/// Mirage five-branch bound, every affected VFS candidate falls back to the
/// ordinary Overlay planner for this boot.
pub fn select_vfs_modules(
    modules: &[Module],
    system_root: &Path,
    capabilities: &BackendCapabilities,
    managed_partitions: &[String],
) -> Result<VfsSelection> {
    let candidates: BTreeSet<String> = modules
        .iter()
        .filter(|module| {
            module.rules.default_mode == MountMode::Overlay && module.rules.paths.is_empty()
        })
        .filter(|module| !active_partitions(module, managed_partitions).is_empty())
        .map(|module| module.id.clone())
        .collect();

    if candidates.is_empty() {
        return Ok(VfsSelection::default());
    }

    if !capabilities.can_use_vfs() {
        return Ok(VfsSelection {
            fallback_module_ids: candidates.into_iter().collect(),
            ..VfsSelection::default()
        });
    }

    let backend = capabilities
        .vfs_fs_type()
        .context("VFS backend marked usable without a filesystem type")?
        .to_string();
    let max_branches = capabilities.vfs_max_branches();
    let max_module_branches = max_branches.saturating_sub(1);
    let candidate_set: HashSet<&str> = candidates.iter().map(String::as_str).collect();
    let mut fallback = BTreeSet::new();

    for partition in managed_partitions {
        let mut vfs_ids = Vec::new();
        let mut other_backend_present = false;

        for module in modules {
            if !module.source_path.join(partition).is_dir() {
                continue;
            }
            if candidate_set.contains(module.id.as_str()) {
                vfs_ids.push(module.id.clone());
            } else {
                other_backend_present = true;
            }
        }

        if vfs_ids.is_empty() {
            continue;
        }

        let target = system_root.join(partition);
        if !target.exists() || other_backend_present || vfs_ids.len() > max_module_branches {
            fallback.extend(vfs_ids);
        }
    }

    let selected: BTreeSet<String> = candidates
        .iter()
        .filter(|id| !fallback.contains(*id))
        .cloned()
        .collect();
    let selected_set: HashSet<&str> = selected.iter().map(String::as_str).collect();
    let mut grouped: BTreeMap<String, Vec<&Module>> = BTreeMap::new();

    for partition in managed_partitions {
        for module in modules {
            if selected_set.contains(module.id.as_str())
                && module.source_path.join(partition).is_dir()
            {
                grouped.entry(partition.clone()).or_default().push(module);
            }
        }
    }

    let mut operations = Vec::new();
    for (partition, mut members) in grouped {
        // Inventory IDs are sorted ascending; both the historical NoMountFS
        // manager and current union semantics expect the last module to win.
        // Put the highest-priority module first in lowerdir order.
        members.sort_by(|a, b| b.id.cmp(&a.id));
        let raw_target = system_root.join(&partition);
        let target = fs::canonicalize(&raw_target)
            .with_context(|| format!("failed to resolve VFS target {}", raw_target.display()))?;
        let lowerdirs = members
            .iter()
            .map(|module| module.source_path.join(&partition))
            .collect::<Vec<PathBuf>>();
        let module_ids = members.iter().map(|module| module.id.clone()).collect();

        operations.push(VfsOperation {
            backend: backend.clone(),
            partition_name: partition,
            target,
            lowerdirs,
            module_ids,
            max_branches,
        });
    }

    Ok(VfsSelection {
        operations,
        module_ids: selected.into_iter().collect(),
        fallback_module_ids: fallback.into_iter().collect(),
    })
}

fn active_partitions(module: &Module, managed_partitions: &[String]) -> Vec<String> {
    managed_partitions
        .iter()
        .filter(|partition| module.source_path.join(partition).is_dir())
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, fs};

    use tempfile::TempDir;

    use super::*;
    use crate::{conf::config::Config, domain::ModuleRules};

    fn module(root: &Path, id: &str, mode: MountMode, partition: &str) -> Module {
        let source = root.join(id);
        fs::create_dir_all(source.join(partition)).unwrap();
        Module {
            id: id.to_string(),
            source_path: source,
            rules: ModuleRules {
                default_mode: mode,
                paths: HashMap::new(),
            },
        }
    }

    #[test]
    fn disabled_vfs_falls_back_without_changing_overlay_semantics() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join("system")).unwrap();
        let modules = vec![module(temp.path(), "a", MountMode::Overlay, "system")];
        let capabilities = BackendCapabilities::detect(&Config::default()).unwrap();
        let selection = select_vfs_modules(
            &modules,
            temp.path(),
            &capabilities,
            &["system".to_string()],
        )
        .unwrap();
        assert!(selection.operations.is_empty());
        assert_eq!(selection.fallback_module_ids, vec!["a"]);
    }
}
