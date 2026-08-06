// Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-only

use std::{collections::HashSet, path::Path};

use anyhow::Result;

use crate::{
    conf::config,
    core::{inventory::Module, runtime_state::MountStatistics},
    mount::magic_mount::{self, MagicMountOptions},
    partitions,
};

pub(super) fn mount_magic(
    modules: &[Module],
    ids: &[String],
    kasumi_fallback_ids: &[String],
    config: &config::Config,
    tempdir: &Path,
) -> Result<(Vec<String>, MountStatistics)> {
    let magic_ws_path = tempdir.join("magic_workspace");

    crate::scoped_log!(
        debug,
        "executor:magic",
        "prepare workspace: path={}",
        magic_ws_path.display()
    );

    if !magic_ws_path.exists() {
        std::fs::create_dir_all(&magic_ws_path)?;
    }

    let module_ids: HashSet<&str> = ids.iter().map(String::as_str).collect();
    let fallback_ids: HashSet<&str> = kasumi_fallback_ids.iter().map(String::as_str).collect();
    let selected_modules: Vec<Module> = modules
        .iter()
        .filter(|module| module_ids.contains(module.id.as_str()))
        .cloned()
        .map(|mut module| {
            if fallback_ids.contains(module.id.as_str()) {
                module.rules = module.rules.with_kasumi_fallback_to_magic();
            }
            module
        })
        .collect();
    let managed_partitions = partitions::managed_partition_names();

    let (mounted_ids, stats) = magic_mount::magic_mount(
        &magic_ws_path,
        &config.moduledir,
        MagicMountOptions {
            mount_source: &config.mountsource,
            managed_partitions: &managed_partitions,
        },
        &selected_modules,
        !config.disable_umount,
    )?;

    crate::scoped_log!(
        debug,
        "executor:magic",
        "complete: requested_modules={}, fallback_modules={}, mounted_modules={}",
        ids.len(),
        kasumi_fallback_ids.len(),
        mounted_ids.len()
    );

    Ok((mounted_ids, stats))
}
