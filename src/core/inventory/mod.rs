// Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-only

pub mod discovery;
pub mod listing;
mod safety;

use std::fs;

use anyhow::Result;
#[cfg(not(feature = "control-plane"))]
use anyhow::bail;
pub(crate) use discovery::validate_module_prop_id;
pub use discovery::{InventorySnapshot, InventorySummary, Module};

#[cfg(not(feature = "control-plane"))]
use crate::domain::MountMode;
use crate::{conf::config::Config, defs, domain::ModuleRules};

pub fn scan(config: &Config) -> Result<Vec<Module>> {
    Ok(scan_snapshot(config)?.modules)
}

pub fn scan_snapshot(config: &Config) -> Result<InventorySnapshot> {
    let mut snapshot = discovery::scan_snapshot(config)?;
    let mut safe_modules = Vec::with_capacity(snapshot.modules.len());
    let mut quarantined = 0usize;

    for module in snapshot.modules {
        match safety::first_blocked_critical_path(&module.source_path, &module.rules) {
            Ok(None) => safe_modules.push(module),
            Ok(Some(path)) => {
                quarantined += 1;
                crate::scoped_log!(
                    error,
                    "inventory:safety",
                    "module quarantined: id={}, path={}, reason=critical_bluetooth_or_radio_payload; add an explicit Ignore rule for the path to mount the safe remainder",
                    module.id,
                    path.display()
                );
            }
            Err(error) => {
                quarantined += 1;
                crate::scoped_log!(
                    error,
                    "inventory:safety",
                    "module quarantined: id={}, reason=critical_path_scan_failed, error={:#}",
                    module.id,
                    error
                );
            }
        }
    }

    if quarantined > 0 {
        crate::scoped_log!(
            warn,
            "inventory:safety",
            "critical payload quarantine complete: quarantined={}, active={}",
            quarantined,
            safe_modules.len()
        );
    }
    snapshot.modules = safe_modules;
    Ok(snapshot)
}

pub fn load_module_rules(config: &Config, module_id: &str) -> Result<ModuleRules> {
    let mut rules = ModuleRules {
        default_mode: config.default_mode.as_mount_mode(),
        ..Default::default()
    };

    if let Some(global_rules) = config.rules.get(module_id) {
        rules.default_mode = global_rules.default_mode;
        rules.paths.extend(global_rules.paths.clone());
    }

    #[cfg(not(feature = "control-plane"))]
    if let Some(marker_mode) = module_mount_mode_marker(&config.moduledir.join(module_id))? {
        rules.default_mode = marker_mode;
    }

    Ok(rules)
}

#[cfg(not(feature = "control-plane"))]
pub fn module_mount_mode_marker(module_path: &std::path::Path) -> Result<Option<MountMode>> {
    let markers = scan_known_markers(module_path)?;
    if markers.overlay && markers.magic {
        bail!(
            "module contains conflicting overlay and magic markers: {}",
            module_path.display()
        );
    }
    if markers.overlay {
        Ok(Some(MountMode::Overlay))
    } else if markers.magic {
        Ok(Some(MountMode::Magic))
    } else {
        Ok(None)
    }
}

pub fn is_reserved_module_dir(id: &str) -> bool {
    matches!(
        id,
        "hybrid-mount"
            | "hybrid_mount"
            | "lost+found"
            | ".git"
            | ".github"
            | ".hg"
            | ".idea"
            | ".svn"
            | ".vscode"
            | "__pycache__"
            | "node_modules"
    )
}

pub fn mount_block_markers(module_path: &std::path::Path) -> Result<Vec<&'static str>> {
    let found = scan_known_markers(module_path)?;
    let mut markers = Vec::new();
    if found.disable {
        markers.push(defs::DISABLE_FILE_NAME);
    }
    if found.remove {
        markers.push(defs::REMOVE_FILE_NAME);
    }
    if found.skip_mount {
        markers.push(defs::SKIP_MOUNT_FILE_NAME);
    }
    Ok(markers)
}

#[derive(Default)]
struct KnownMarkers {
    disable: bool,
    remove: bool,
    skip_mount: bool,
    overlay: bool,
    magic: bool,
}

fn scan_known_markers(module_path: &std::path::Path) -> Result<KnownMarkers> {
    let mut found = KnownMarkers::default();
    let entries = fs::read_dir(module_path)?;

    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        found.disable |= name == defs::DISABLE_FILE_NAME;
        found.remove |= name == defs::REMOVE_FILE_NAME;
        found.skip_mount |= name == defs::SKIP_MOUNT_FILE_NAME;
        found.overlay |= name == "overlay";
        found.magic |= name == "magic";
    }

    Ok(found)
}

pub fn has_mount_block_marker(module_path: &std::path::Path) -> Result<bool> {
    Ok(!mount_block_markers(module_path)?.is_empty())
}

#[cfg(all(test, not(feature = "control-plane")))]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;
    use crate::domain::DefaultMode;

    #[test]
    fn module_mount_mode_marker_detects_mode_files() {
        let temp = TempDir::new().unwrap();
        let module_path = temp.path().join("module");
        fs::create_dir_all(&module_path).unwrap();

        assert_eq!(module_mount_mode_marker(&module_path).unwrap(), None);

        fs::write(module_path.join("magic"), b"").unwrap();
        assert_eq!(
            module_mount_mode_marker(&module_path).unwrap(),
            Some(MountMode::Magic)
        );
    }

    #[test]
    fn module_mount_mode_marker_rejects_conflicting_markers() {
        let temp = TempDir::new().unwrap();
        let module_path = temp.path().join("module");
        fs::create_dir_all(&module_path).unwrap();
        fs::write(module_path.join("overlay"), b"").unwrap();
        fs::write(module_path.join("magic"), b"").unwrap();

        assert!(module_mount_mode_marker(&module_path).is_err());
    }

    #[test]
    fn module_mount_mode_marker_ignores_kasumi_for_nano() {
        let temp = TempDir::new().unwrap();
        let module_path = temp.path().join("module");
        fs::create_dir_all(&module_path).unwrap();
        fs::write(module_path.join("kasumi"), b"").unwrap();

        assert_eq!(module_mount_mode_marker(&module_path).unwrap(), None);
    }

    #[test]
    fn load_module_rules_uses_mode_marker_for_nano_default() {
        let temp = TempDir::new().unwrap();
        let module_path = temp.path().join("module");
        fs::create_dir_all(&module_path).unwrap();
        fs::write(module_path.join("magic"), b"").unwrap();

        let mut config = Config {
            moduledir: temp.path().to_path_buf(),
            default_mode: DefaultMode::Overlay,
            ..Config::default()
        };
        config.rules.insert(
            "module".to_string(),
            ModuleRules {
                default_mode: MountMode::Overlay,
                ..Default::default()
            },
        );

        assert_eq!(
            load_module_rules(&config, "module").unwrap().default_mode,
            MountMode::Magic
        );
    }
}

#[cfg(test)]
mod quarantine_tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;
    use crate::domain::MountMode;

    fn create_module(root: &std::path::Path, id: &str) -> std::path::PathBuf {
        let module = root.join(id);
        fs::create_dir_all(&module).unwrap();
        fs::write(module.join("module.prop"), format!("id={id}\n")).unwrap();
        module
    }

    #[test]
    fn scan_quarantines_only_the_module_with_critical_payload() {
        let temp = TempDir::new().unwrap();
        let safe = create_module(temp.path(), "safe");
        fs::create_dir_all(safe.join("system/app")).unwrap();
        fs::write(safe.join("system/app/example.apk"), b"safe").unwrap();

        let critical = create_module(temp.path(), "critical");
        fs::create_dir_all(critical.join("vendor/firmware")).unwrap();
        fs::write(critical.join("vendor/firmware/modem.mbn"), b"blocked").unwrap();

        let config = Config {
            moduledir: temp.path().to_path_buf(),
            ..Config::default()
        };
        let snapshot = scan_snapshot(&config).unwrap();

        assert_eq!(snapshot.modules.len(), 1);
        assert_eq!(snapshot.modules[0].id, "safe");
    }

    #[test]
    fn scan_accepts_module_when_critical_subtree_is_explicitly_ignored() {
        let temp = TempDir::new().unwrap();
        let module = create_module(temp.path(), "critical");
        fs::create_dir_all(module.join("vendor/firmware")).unwrap();
        fs::write(module.join("vendor/firmware/modem.mbn"), b"ignored").unwrap();

        let mut config = Config {
            moduledir: temp.path().to_path_buf(),
            ..Config::default()
        };
        config.rules.insert(
            "critical".to_string(),
            ModuleRules {
                default_mode: MountMode::Overlay,
                paths: [("vendor/firmware".to_string(), MountMode::Ignore)]
                    .into_iter()
                    .collect(),
            },
        );

        let snapshot = scan_snapshot(&config).unwrap();
        assert_eq!(snapshot.modules.len(), 1);
        assert_eq!(snapshot.modules[0].id, "critical");
    }
}
