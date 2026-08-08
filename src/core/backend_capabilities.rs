// Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-only

use std::fs;

use anyhow::Result;

use crate::conf::{config::Config, schema::VfsBackendPreference};
#[cfg(feature = "kasumi")]
use crate::sys::kasumi;

#[derive(Debug, Clone, Default)]
pub struct BackendCapabilities {
    kasumi_status: String,
    kasumi_usable: bool,
    vfs_status: String,
    vfs_usable: bool,
    vfs_fs_type: Option<String>,
    vfs_max_branches: usize,
}

impl BackendCapabilities {
    pub fn detect(config: &Config) -> Result<Self> {
        let (vfs_status, vfs_usable, vfs_fs_type) = detect_vfs(config);
        let vfs_max_branches = config.vfs.effective_max_branches();

        #[cfg(not(feature = "kasumi"))]
        {
            Ok(Self {
                kasumi_status: "disabled".to_string(),
                kasumi_usable: false,
                vfs_status,
                vfs_usable,
                vfs_fs_type,
                vfs_max_branches,
            })
        }

        #[cfg(feature = "kasumi")]
        {
            let (kasumi_status, kasumi_usable) = if !config.kasumi.enabled {
                ("disabled".to_string(), false)
            } else {
                match kasumi::check_status() {
                    Ok(status) => (
                        kasumi::status_name(status).to_string(),
                        matches!(status, kasumi::KasumiStatus::Available),
                    ),
                    Err(error) => {
                        crate::scoped_log!(
                            warn,
                            "backend_capabilities",
                            "kasumi probe failed; backend marked unavailable: error={:#}",
                            error
                        );
                        ("probe_error".to_string(), false)
                    }
                }
            };

            Ok(Self {
                kasumi_status,
                kasumi_usable,
                vfs_status,
                vfs_usable,
                vfs_fs_type,
                vfs_max_branches,
            })
        }
    }

    pub fn can_use_kasumi(&self) -> bool {
        self.kasumi_usable
    }

    pub fn kasumi_status(&self) -> &str {
        &self.kasumi_status
    }

    pub fn can_use_vfs(&self) -> bool {
        self.vfs_usable
    }

    pub fn vfs_status(&self) -> &str {
        &self.vfs_status
    }

    pub fn vfs_fs_type(&self) -> Option<&str> {
        self.vfs_fs_type.as_deref()
    }

    pub fn vfs_max_branches(&self) -> usize {
        self.vfs_max_branches
    }
}

fn detect_vfs(config: &Config) -> (String, bool, Option<String>) {
    if !config.vfs.enabled {
        return ("disabled".to_string(), false, None);
    }

    let filesystems = match fs::read_to_string("/proc/filesystems") {
        Ok(content) => content,
        Err(error) => {
            crate::scoped_log!(
                warn,
                "backend_capabilities",
                "vfs filesystem probe failed: error={:#}",
                error
            );
            return ("probe_error".to_string(), false, None);
        }
    };

    select_vfs_backend(&filesystems, config.vfs.backend)
        .map(|fs_type| (fs_type.clone(), true, Some(fs_type)))
        .unwrap_or_else(|| ("unavailable".to_string(), false, None))
}

fn select_vfs_backend(filesystems: &str, preference: VfsBackendPreference) -> Option<String> {
    let available = |name: &str| {
        filesystems
            .lines()
            .filter_map(|line| line.split_whitespace().last())
            .any(|entry| entry == name)
    };

    match preference {
        VfsBackendPreference::Auto => {
            if available("mirage") {
                Some("mirage".to_string())
            } else if available("nomountfs") {
                Some("nomountfs".to_string())
            } else {
                None
            }
        }
        VfsBackendPreference::Mirage => available("mirage").then(|| "mirage".to_string()),
        VfsBackendPreference::Nomountfs => {
            available("nomountfs").then(|| "nomountfs".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_vfs_does_not_probe_or_activate() {
        let config = Config::default();
        let capabilities = BackendCapabilities::detect(&config).unwrap();
        assert!(!capabilities.can_use_vfs());
        assert_eq!(capabilities.vfs_status(), "disabled");
    }

    #[test]
    fn auto_prefers_current_mirage_over_legacy_nomountfs() {
        let filesystems = "nodev\tnomountfs\nnodev\tmirage\n";
        assert_eq!(
            select_vfs_backend(filesystems, VfsBackendPreference::Auto).as_deref(),
            Some("mirage")
        );
    }

    #[test]
    fn explicit_backend_never_silently_switches() {
        let filesystems = "nodev\tnomountfs\n";
        assert!(select_vfs_backend(filesystems, VfsBackendPreference::Mirage).is_none());
    }
}
