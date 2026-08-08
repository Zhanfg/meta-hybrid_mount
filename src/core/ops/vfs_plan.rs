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
    mount::zeromount,
};

#[derive(Debug, Default)]
pub struct VfsSelection {
    pub backend: Option<String>,
    pub operations: Vec<VfsOperation>,
    pub module_ids: HashSet<String>,
    pub fallback_module_ids: Vec<String>,
}

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

    if candidates.is_empty() || capabilities.vfs_status() == "disabled" {
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
        .context("VFS backend marked usable without a backend name")?
        .to_string();
    let max_branches = capabilities.vfs_max_branches();
    let max_module_branches = max_branches.saturating_sub(1);
    let candidate_set: HashSet<&str> = candidates.iter().map(String::as_str).collect();
    let mut fallback = BTreeSet::new();

    if backend == "zeromount" {
        for module in modules {
            if candidate_set.contains(module.id.as_str())
                && !zeromount::module_preflight_supported(module, managed_partitions)?
            {
                fallback.insert(module.id.clone());
            }
        }
    }

    for partition in managed_partitions {
        let mut vfs_ids = Vec::new();
        let mut other_backend_present = false;
        for module in modules {
            if !module.source_path.join(partition).is_dir() {
                continue;
            }
            if candidate_set.contains(module.id.as_str()) && !fallback.contains(&module.id) {
                vfs_ids.push(module.id.clone());
            } else {
                other_backend_present = true;
            }
        }
        if vfs_ids.is_empty() {
            continue;
        }
        let target = system_root.join(partition);
        let unsafe_partition = if backend == "zeromount" {
            // First safe ZeroMount generation: only one REHYBIRD VFS module may
            // own a partition. This avoids ambiguous new-directory merging and
            // duplicate virtual rules between modules.
            !target.exists() || other_backend_present || vfs_ids.len() != 1
        } else {
            !target.exists() || other_backend_present || vfs_ids.len() > max_module_branches
        };
        if unsafe_partition {
            fallback.extend(vfs_ids);
        }
    }

    let selected: BTreeSet<String> = candidates
        .iter()
        .filter(|id| !fallback.contains(*id))
        .cloned()
        .collect();
    let selected_set: HashSet<&str> = selected.iter().map(String::as_str).collect();
    let mut operations = Vec::new();

    if backend != "zeromount" {
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
        for (partition, mut members) in grouped {
            members.sort_by(|a, b| b.id.cmp(&a.id));
            let raw_target = system_root.join(&partition);
            let target = fs::canonicalize(&raw_target).with_context(|| {
                format!("failed to resolve VFS target {}", raw_target.display())
            })?;
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
    }

    Ok(VfsSelection {
        backend: (!selected.is_empty()).then_some(backend),
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
    use crate::domain::ModuleRules;

    fn capabilities(
        status: &str,
        usable: bool,
        backend: Option<&str>,
        max_branches: usize,
    ) -> BackendCapabilities {
        BackendCapabilities::for_vfs_test(status, usable, backend, max_branches)
    }

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
    fn disabled_vfs_is_not_reported_as_fallback() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join("system")).unwrap();
        let modules = vec![module(temp.path(), "a", MountMode::Overlay, "system")];
        let selection = select_vfs_modules(
            &modules,
            temp.path(),
            &capabilities("disabled", false, None, 5),
            &["system".to_string()],
        )
        .unwrap();
        assert!(selection.operations.is_empty());
        assert!(selection.fallback_module_ids.is_empty());
    }

    #[test]
    fn union_branch_overflow_falls_back_before_mount() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join("system")).unwrap();
        let modules = ["a", "b", "c", "d", "e"]
            .into_iter()
            .map(|id| module(temp.path(), id, MountMode::Overlay, "system"))
            .collect::<Vec<_>>();
        let selection = select_vfs_modules(
            &modules,
            temp.path(),
            &capabilities("mirage", true, Some("mirage"), 5),
            &["system".to_string()],
        )
        .unwrap();
        assert!(selection.operations.is_empty());
        assert_eq!(selection.fallback_module_ids, vec!["a", "b", "c", "d", "e"]);
    }

    #[test]
    fn zeromount_rejects_shared_partition_before_runtime() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join("system")).unwrap();
        let modules = vec![
            module(temp.path(), "a", MountMode::Overlay, "system"),
            module(temp.path(), "b", MountMode::Overlay, "system"),
        ];
        let selection = select_vfs_modules(
            &modules,
            temp.path(),
            &capabilities("zeromount_v1", true, Some("zeromount"), 5),
            &["system".to_string()],
        )
        .unwrap();
        assert!(selection.module_ids.is_empty());
        assert_eq!(selection.fallback_module_ids, vec!["a", "b"]);
    }

    #[test]
    fn legacy_priority_maps_to_first_lowerdir_wins() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join("system")).unwrap();
        let modules = vec![
            module(temp.path(), "a", MountMode::Overlay, "system"),
            module(temp.path(), "z", MountMode::Overlay, "system"),
        ];
        let selection = select_vfs_modules(
            &modules,
            temp.path(),
            &capabilities("mirage", true, Some("mirage"), 5),
            &["system".to_string()],
        )
        .unwrap();
        assert_eq!(selection.operations.len(), 1);
        assert_eq!(selection.operations[0].module_ids, vec!["z", "a"]);
    }
}
