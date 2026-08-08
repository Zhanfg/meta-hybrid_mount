// Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-only

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[cfg(feature = "kasumi")]
use crate::mount::kasumi;
#[cfg(feature = "control-plane")]
use crate::sys::fs::xattr;
use crate::{
    conf::config::Config,
    core::{inventory::InventorySummary, ops::executor::ExecutionResult},
    defs,
    sys::fs::atomic_write,
};

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct MountStatistics {
    pub total_mounts: usize,
    pub successful_mounts: usize,
    pub failed_mounts: usize,
    pub tmpfs_created: usize,
    pub files_mounted: usize,
    pub dirs_mounted: usize,
    pub symlinks_created: usize,
    pub overlayfs_mounts: usize,
    #[serde(default)]
    pub vfs_mounts: usize,
    pub ignored_entries: usize,
}

impl MountStatistics {
    pub fn record_file(&mut self) { self.total_mounts += 1; self.successful_mounts += 1; self.files_mounted += 1; }
    pub fn record_dir(&mut self) { self.total_mounts += 1; self.successful_mounts += 1; self.dirs_mounted += 1; }
    pub fn record_symlink(&mut self) { self.total_mounts += 1; self.successful_mounts += 1; self.symlinks_created += 1; }
    pub fn record_failed(&mut self) { self.total_mounts += 1; self.failed_mounts += 1; }
    pub fn record_tmpfs(&mut self) { self.tmpfs_created += 1; }
    pub fn record_overlay_mount(&mut self) { self.total_mounts += 1; self.successful_mounts += 1; self.overlayfs_mounts += 1; }
    pub fn record_vfs_mount(&mut self) { self.total_mounts += 1; self.successful_mounts += 1; self.vfs_mounts += 1; }
    pub fn record_ignored(&mut self) { self.ignored_entries += 1; }

    #[cfg(feature = "control-plane")]
    pub fn success_rate(&self) -> f64 {
        if self.total_mounts == 0 { 0.0 } else { self.successful_mounts as f64 * 100.0 / self.total_mounts as f64 }
    }

    pub fn merge(&mut self, other: &Self) {
        self.total_mounts += other.total_mounts;
        self.successful_mounts += other.successful_mounts;
        self.failed_mounts += other.failed_mounts;
        self.tmpfs_created += other.tmpfs_created;
        self.files_mounted += other.files_mounted;
        self.dirs_mounted += other.dirs_mounted;
        self.symlinks_created += other.symlinks_created;
        self.overlayfs_mounts += other.overlayfs_mounts;
        self.vfs_mounts += other.vfs_mounts;
        self.ignored_entries += other.ignored_entries;
    }
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct ModuleModeStats {
    pub overlayfs: usize,
    pub magicmount: usize,
    pub kasumi: usize,
    #[serde(default)]
    pub vfs: usize,
    pub blacklisted: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct KasumiRuntimeInfo {
    pub status: String,
    pub available: bool,
    pub kernel_supported: bool,
    pub lkm_loaded: bool,
    pub lkm_autoload: bool,
    pub lkm_kmi_override: String,
    pub lkm_current_kmi: String,
    pub lkm_dir: PathBuf,
    pub protocol_version: Option<i32>,
    pub feature_bits: Option<i32>,
    pub feature_names: Vec<String>,
    pub hooks: Vec<String>,
    pub rule_count: usize,
    pub user_hide_rule_count: usize,
    pub mirror_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DaemonRuntimeInfo {
    pub alive: bool,
    pub socket_path: String,
    pub last_refresh_ts: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RuntimeState {
    pub timestamp: u64,
    pub pid: u32,
    pub storage_mode: String,
    pub mount_point: PathBuf,
    #[serde(default)]
    pub vfs_modules: Vec<String>,
    #[serde(default)]
    pub vfs_backend: Option<String>,
    #[serde(default)]
    pub vfs_fallback_modules: Vec<String>,
    pub overlay_modules: Vec<String>,
    pub magic_modules: Vec<String>,
    pub kasumi_modules: Vec<String>,
    pub custom_mounts: Vec<String>,
    pub skip_mount_modules: Vec<String>,
    pub blacklisted_modules: Vec<String>,
    pub active_mounts: Vec<String>,
    #[cfg(feature = "control-plane")]
    pub tmpfs_xattr_supported: bool,
    pub mount_stats: MountStatistics,
    pub mode_stats: ModuleModeStats,
    pub kasumi: KasumiRuntimeInfo,
    pub daemon: DaemonRuntimeInfo,
    #[serde(skip)]
    cached_status_value: Option<serde_json::Value>,
}

impl RuntimeState {
    #[cfg(feature = "control-plane")]
    pub fn status_value(&mut self) -> serde_json::Result<&serde_json::Value> {
        if self.cached_status_value.is_none() {
            self.cached_status_value = Some(serde_json::to_value(&*self)?);
        }
        Ok(self.cached_status_value.as_ref().expect("cached status populated"))
    }

    fn invalidate_cache(&mut self) { self.cached_status_value = None; }

    pub fn save(&self) -> Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        match std::fs::read_to_string(defs::STATE_FILE) {
            Ok(existing) if existing == json => return Ok(()),
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }
        atomic_write(defs::STATE_FILE, json.as_bytes())?;
        Ok(())
    }

    pub fn build_from_execution(
        config: &Config,
        storage_mode: crate::core::storage::StorageMode,
        mount_point: &Path,
        result: &ExecutionResult,
        inventory: &InventorySummary,
    ) -> Result<Self> {
        #[cfg(feature = "kasumi")]
        let kasumi = kasumi::collect_runtime_info(config)?;
        #[cfg(not(feature = "kasumi"))]
        let kasumi = { let _ = config; KasumiRuntimeInfo::default() };

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|err| anyhow::anyhow!("system clock is before the Unix epoch: {err}"))?
            .as_secs();
        #[cfg(feature = "control-plane")]
        let tmpfs_xattr_supported = xattr::is_overlay_xattr_supported()?;

        let mut state = Self {
            timestamp,
            pid: std::process::id(),
            storage_mode: storage_mode.as_str().to_owned(),
            mount_point: mount_point.to_path_buf(),
            vfs_modules: result.vfs_module_ids.clone(),
            vfs_backend: result.vfs_backend.clone(),
            vfs_fallback_modules: result.vfs_fallback_module_ids.clone(),
            overlay_modules: result.overlay_module_ids.clone(),
            magic_modules: result.magic_module_ids.clone(),
            kasumi_modules: {
                #[cfg(feature = "kasumi")]
                { result.kasumi_module_ids.clone() }
                #[cfg(not(feature = "kasumi"))]
                { Vec::new() }
            },
            custom_mounts: result.custom_mount_targets.clone(),
            skip_mount_modules: inventory.skip_mount_modules.clone(),
            blacklisted_modules: inventory.blacklisted_modules.clone(),
            active_mounts: collect_active_mounts(result),
            #[cfg(feature = "control-plane")]
            tmpfs_xattr_supported,
            mount_stats: result.mount_stats.clone(),
            mode_stats: collect_mode_stats(result),
            kasumi,
            daemon: DaemonRuntimeInfo::default(),
            cached_status_value: None,
        };
        state.mode_stats.blacklisted = state.blacklisted_modules.len();
        state.invalidate_cache();
        Ok(state)
    }

    pub fn mounted_module_ids(&self) -> HashSet<&str> {
        self.vfs_modules
            .iter()
            .chain(self.overlay_modules.iter())
            .chain(self.magic_modules.iter())
            .chain(self.kasumi_modules.iter())
            .map(String::as_str)
            .collect()
    }

    #[cfg(feature = "control-plane")]
    pub fn set_daemon_state(&mut self, alive: bool, socket_path: impl Into<String>) -> Result<()> {
        self.daemon.alive = alive;
        self.daemon.socket_path = socket_path.into();
        self.daemon.last_refresh_ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|err| anyhow::anyhow!("system clock is before the Unix epoch: {err}"))?
            .as_secs();
        self.invalidate_cache();
        Ok(())
    }

    pub fn load() -> Result<Self> {
        let content = fs::read_to_string(defs::STATE_FILE)?;
        Ok(serde_json::from_str(&content)?)
    }
}

fn collect_mode_stats(result: &ExecutionResult) -> ModuleModeStats {
    ModuleModeStats {
        overlayfs: result.overlay_module_ids.len(),
        magicmount: result.magic_module_ids.len(),
        kasumi: result.kasumi_count(),
        vfs: result.vfs_module_ids.len(),
        blacklisted: 0,
    }
}

fn collect_active_mounts(result: &ExecutionResult) -> Vec<String> {
    let mut active_mounts = result.overlay_partitions.clone();
    active_mounts.extend(result.vfs_partitions.iter().map(|p| format!("vfs:{p}")));
    if !result.custom_mount_targets.is_empty() { active_mounts.push("custom-bind".to_string()); }
    if result.kasumi_runtime_enabled { active_mounts.push("kasumi".to_string()); }
    active_mounts.sort();
    active_mounts.dedup();
    active_mounts
}
