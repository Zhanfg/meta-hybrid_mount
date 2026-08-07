// Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-only

use std::{
    collections::HashSet,
    fs,
    os::unix::fs::{FileTypeExt, MetadataExt},
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use super::common::build_managed_partitions;
use crate::{
    conf::config,
    core::{
        inventory::Module,
        ops::plan::{KasumiAddRule, KasumiMergeRule, MountPlan},
    },
    defs,
    domain::MountMode,
    utils,
};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub(super) struct CompiledRules {
    pub(super) add_rules: Vec<KasumiAddRule>,
    pub(super) merge_rules: Vec<KasumiMergeRule>,
    pub(super) hide_rules: Vec<String>,
}

fn mirror_module_root(config: &config::Config, module: &Module) -> Result<PathBuf> {
    let mirror_root = fs::canonicalize(&config.kasumi.mirror_path).with_context(|| {
        format!(
            "failed to resolve Kasumi mirror root {}",
            config.kasumi.mirror_path.display()
        )
    })?;
    let requested_module_root = config.kasumi.mirror_path.join(&module.id);
    let module_root = fs::canonicalize(&requested_module_root).with_context(|| {
        format!(
            "missing or invalid Kasumi mirror content for module {} at {}",
            module.id,
            requested_module_root.display()
        )
    })?;

    if module_root == mirror_root || !module_root.starts_with(&mirror_root) {
        bail!(
            "Kasumi mirror module root escaped the configured mirror: module={}, root={}, mirror={}",
            module.id,
            module_root.display(),
            mirror_root.display()
        );
    }
    Ok(module_root)
}

fn build_dtype(path: &Path) -> Result<(i32, bool)> {
    let metadata = fs::symlink_metadata(path).with_context(|| {
        format!(
            "failed to read metadata for kasumi source {}",
            path.display()
        )
    })?;
    let file_type = metadata.file_type();

    if file_type.is_char_device() && metadata.rdev() == 0 {
        return Ok((libc::DT_UNKNOWN as i32, true));
    }

    let d_type = if file_type.is_file() {
        libc::DT_REG
    } else if file_type.is_symlink() {
        libc::DT_LNK
    } else if file_type.is_dir() {
        libc::DT_DIR
    } else if file_type.is_block_device() {
        libc::DT_BLK
    } else if file_type.is_char_device() {
        libc::DT_CHR
    } else if file_type.is_fifo() {
        libc::DT_FIFO
    } else if file_type.is_socket() {
        libc::DT_SOCK
    } else {
        libc::DT_UNKNOWN
    };

    Ok((d_type as i32, false))
}

pub(super) fn log_compiled_rule_summary(compiled: &CompiledRules, user_hide_paths: &[PathBuf]) {
    crate::scoped_log!(
        debug,
        "mount:kasumi",
        "compiled rules: add_rules={}, merge_rules={}, hide_rules={}, user_hide_rules={}",
        compiled.add_rules.len(),
        compiled.merge_rules.len(),
        compiled.hide_rules.len(),
        user_hide_paths.len()
    );
}

fn relative_mode(module: &Module, relative: &Path) -> MountMode {
    let relative_str = relative.to_string_lossy();
    module.rules.get_mode(relative_str.as_ref())
}

pub(super) fn virtual_target_is_managed(
    target: &Path,
    managed_partitions: &HashSet<String>,
) -> bool {
    let Ok(relative) = target.strip_prefix("/") else {
        return false;
    };
    let Some(Component::Normal(partition)) = relative.components().next() else {
        return false;
    };
    partition
        .to_str()
        .is_some_and(|partition| managed_partitions.contains(partition))
}

pub(super) fn compile_rules(
    modules: &[Module],
    plan: &MountPlan,
    config: &config::Config,
) -> Result<CompiledRules> {
    let system_root = Path::new("/");
    let managed_partitions = build_managed_partitions(config);
    let active_ids: HashSet<&str> = plan.kasumi_module_ids.iter().map(String::as_str).collect();
    let mut compiled = CompiledRules::default();
    let mut managed_partition_list: Vec<String> = managed_partitions.iter().cloned().collect();
    managed_partition_list.sort();

    for module in modules.iter().rev() {
        if !active_ids.contains(module.id.as_str()) {
            continue;
        }

        let module_root = mirror_module_root(config, module)?;
        let mut scanned_partition_roots: HashSet<PathBuf> = HashSet::new();
        let mut symlink_directory_skips = 0usize;

        for partition_name in &managed_partition_list {
            let requested_partition_root = module_root.join(partition_name);
            if !requested_partition_root.is_dir() {
                continue;
            }
            let partition_root =
                fs::canonicalize(&requested_partition_root).with_context(|| {
                    format!(
                        "failed to resolve Kasumi partition root for module {}: {}",
                        module.id,
                        requested_partition_root.display()
                    )
                })?;
            if partition_root == module_root || !partition_root.starts_with(&module_root) {
                bail!(
                    "Kasumi partition source escaped the module mirror: module={}, partition={}, root={}, module_root={}",
                    module.id,
                    partition_name,
                    partition_root.display(),
                    module_root.display()
                );
            }
            if !scanned_partition_roots.insert(partition_root.clone()) {
                crate::scoped_log!(
                    debug,
                    "mount:kasumi",
                    "partition root dedupe: module={}, partition={}, root={}",
                    module.id,
                    partition_name,
                    partition_root.display()
                );
                continue;
            }

            let mut iterator = WalkDir::new(&partition_root)
                .follow_links(false)
                .into_iter();

            while let Some(entry_result) = iterator.next() {
                let entry = entry_result.with_context(|| {
                    format!(
                        "failed to walk Kasumi module {} partition {}",
                        module.id, partition_name
                    )
                })?;

                if entry.depth() == 0 {
                    continue;
                }

                let path = entry.path();
                let relative_in_partition =
                    path.strip_prefix(&partition_root).with_context(|| {
                        format!(
                            "Kasumi path {} is outside partition root {}",
                            path.display(),
                            partition_root.display()
                        )
                    })?;
                let relative = Path::new(partition_name).join(relative_in_partition);

                if !matches!(relative_mode(module, &relative), MountMode::Kasumi) {
                    continue;
                }

                if utils::path_file_name_eq(path, defs::REPLACE_DIR_FILE_NAME) {
                    continue;
                }

                let resolved_virtual_path =
                    utils::resolve_path_with_root(system_root, &Path::new("/").join(&relative))?;
                if !virtual_target_is_managed(&resolved_virtual_path, &managed_partitions) {
                    bail!(
                        "Kasumi target escaped managed partitions: module={}, source={}, target={}",
                        module.id,
                        path.display(),
                        resolved_virtual_path.display()
                    );
                }
                let target_key = resolved_virtual_path.display().to_string();

                if entry.file_type().is_dir() {
                    if resolved_virtual_path.is_dir() {
                        compiled.merge_rules.push(KasumiMergeRule {
                            target: target_key,
                            source: path.to_path_buf(),
                        });
                        iterator.skip_current_dir();
                    }
                    continue;
                }

                if entry.file_type().is_symlink()
                    && resolved_virtual_path.exists()
                    && resolved_virtual_path.is_dir()
                {
                    symlink_directory_skips += 1;
                    continue;
                }

                let (file_type, hide_only) = build_dtype(path)?;
                if hide_only {
                    compiled.hide_rules.push(target_key);
                    continue;
                }

                compiled.add_rules.push(KasumiAddRule {
                    target: target_key,
                    source: path.to_path_buf(),
                    file_type,
                });
            }
        }

        if symlink_directory_skips > 0 {
            crate::scoped_log!(
                warn,
                "mount:kasumi",
                "symlink skip summary: module={}, reason=directory_target, count={}",
                module.id,
                symlink_directory_skips
            );
        }
    }

    Ok(compiled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kasumi_targets_must_remain_in_managed_partitions() {
        let managed = HashSet::from(["system".to_string(), "vendor".to_string()]);

        assert!(virtual_target_is_managed(
            Path::new("/system/etc/file"),
            &managed
        ));
        assert!(virtual_target_is_managed(
            Path::new("/vendor/lib64/file"),
            &managed
        ));
        assert!(!virtual_target_is_managed(
            Path::new("/data/local/tmp/file"),
            &managed
        ));
        assert!(!virtual_target_is_managed(Path::new("/"), &managed));
        assert!(!virtual_target_is_managed(
            Path::new("relative/path"),
            &managed
        ));
    }
}
