// Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-only

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};

use crate::{
    conf::{config::Config, loader},
    defs,
    mount::kasumi as kasumi_mount,
    sys::{fs::atomic_write, kasumi},
    utils,
};

fn validate_hide_path_shape(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        bail!("hide path must be absolute: {}", path.display());
    }

    let normalized = utils::normalize_path(path);
    if normalized != path {
        bail!(
            "hide path must already be normalized: original={}, normalized={}",
            path.display(),
            normalized.display()
        );
    }
    if normalized == Path::new("/") {
        bail!("refusing to use the filesystem root as a hide rule");
    }
    Ok(normalized)
}

fn validate_hide_path(path: &Path) -> Result<PathBuf> {
    let normalized = validate_hide_path_shape(path)?;
    crate::path_safety::ensure_kasumi_target_allowed(&normalized)
        .context("refusing to persist a radio/firmware/system-critical hide rule")?;
    Ok(normalized)
}

fn load_user_hide_rules_with<F>(path: &Path, mut validate: F) -> Result<Vec<PathBuf>>
where
    F: FnMut(&Path) -> Result<PathBuf>,
{
    if !path.exists() {
        return Ok(Vec::new());
    }

    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read user hide rules file {}", path.display()))?;
    let values: Vec<String> = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse user hide rules file {}", path.display()))?;

    let mut seen = HashSet::new();
    let mut rules = Vec::with_capacity(values.len());
    for value in values {
        let rule = validate(Path::new(&value))
            .with_context(|| format!("invalid user hide rule in {}: {value}", path.display()))?;
        if seen.insert(rule.clone()) {
            rules.push(rule);
        }
    }
    Ok(rules)
}

fn load_user_hide_rules_from(path: &Path) -> Result<Vec<PathBuf>> {
    load_user_hide_rules_with(path, validate_hide_path)
}

fn load_user_hide_rules_for_removal(path: &Path) -> Result<Vec<PathBuf>> {
    load_user_hide_rules_with(path, validate_hide_path_shape)
}

fn save_user_hide_rules_to(path: &Path, rules: &[PathBuf]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create user hide rules parent directory {}",
                parent.display()
            )
        })?;
    }

    let values: Vec<String> = rules
        .iter()
        .map(|rule| rule.to_string_lossy().into_owned())
        .collect();
    let payload =
        serde_json::to_string_pretty(&values).context("failed to serialize user hide rules")?;
    atomic_write(path, payload.as_bytes())
        .with_context(|| format!("failed to write user hide rules file {}", path.display()))?;
    Ok(())
}

fn load_runtime_config() -> Result<Config> {
    let config = Config::load_from_file(Path::new(defs::CONFIG_FILE)).with_context(|| {
        format!(
            "failed to load runtime config for user hide update: {}",
            defs::CONFIG_FILE
        )
    })?;
    loader::load_module_blacklist(config)
}

fn commit_user_hide_rules(previous: &[PathBuf], updated: &[PathBuf]) -> Result<()> {
    let config = load_runtime_config()?;
    save_user_hide_rules(updated)?;

    if !config.kasumi.enabled {
        return Ok(());
    }

    if let Err(update_error) = kasumi_mount::apply_runtime_config(&config) {
        if let Err(save_error) = save_user_hide_rules_to(Path::new(defs::USER_HIDE_RULES_FILE), previous)
        {
            bail!(
                "user hide runtime update failed and rule file rollback also failed: update={update_error:#}; save_rollback={save_error:#}"
            );
        }

        return match kasumi_mount::apply_runtime_config(&config) {
            Ok(_) => Err(update_error.context(
                "user hide runtime update failed; previous rules and runtime were restored",
            )),
            Err(rollback_error) => bail!(
                "user hide runtime update and rollback both failed: update={update_error:#}; rollback={rollback_error:#}"
            ),
        };
    }

    Ok(())
}

pub fn load_user_hide_rules() -> Result<Vec<PathBuf>> {
    load_user_hide_rules_from(Path::new(defs::USER_HIDE_RULES_FILE))
}

pub fn save_user_hide_rules(rules: &[PathBuf]) -> Result<()> {
    for rule in rules {
        validate_hide_path(rule)?;
    }
    save_user_hide_rules_to(Path::new(defs::USER_HIDE_RULES_FILE), rules)
}

pub fn user_hide_rule_count() -> Result<usize> {
    Ok(load_user_hide_rules()?.len())
}

pub fn add_user_hide_rule(path: &Path) -> Result<bool> {
    let path = validate_hide_path(path)?;
    let previous = load_user_hide_rules()?;
    if previous.iter().any(|rule| rule == &path) {
        return Ok(false);
    }

    let mut updated = previous.clone();
    updated.push(path);
    commit_user_hide_rules(&previous, &updated)?;

    // The dispatcher issues one final direct hide ioctl when this returns
    // true. Kasumi deduplicates both the hide path and its parent inject rule,
    // so the call is idempotent while preserving the correct API result.
    Ok(true)
}

pub fn remove_user_hide_rule(path: &Path) -> Result<bool> {
    let path = validate_hide_path_shape(path)?;
    let rules_path = Path::new(defs::USER_HIDE_RULES_FILE);
    let previous = load_user_hide_rules_for_removal(rules_path)?;
    let mut updated = previous.clone();
    updated.retain(|rule| rule != &path);

    if updated.len() == previous.len() {
        return Ok(false);
    }

    // Removal must stay available even when the legacy file contains another
    // now-forbidden target. Persist the reduced set first; runtime application
    // remains fail-closed until every protected legacy rule has been removed.
    save_user_hide_rules_to(rules_path, &updated)?;
    if updated.iter().all(|rule| validate_hide_path(rule).is_ok()) {
        let config = load_runtime_config()?;
        if config.kasumi.enabled
            && let Err(update_error) = kasumi_mount::apply_runtime_config(&config)
        {
            return match kasumi_mount::rollback_runtime() {
                Ok(()) => Err(update_error.context(
                    "failed to rebuild Kasumi runtime after hide-rule removal; stale runtime was disabled and cleared",
                )),
                Err(cleanup_error) => bail!(
                    "hide-rule removal rebuild failed and stale Kasumi runtime cleanup also failed: update={update_error:#}; cleanup={cleanup_error:#}"
                ),
            };
        }
    }
    Ok(true)
}

pub fn apply_user_hide_rules() -> Result<usize> {
    let rules = load_user_hide_rules()?;
    let config = load_runtime_config()?;
    if !config.kasumi.enabled {
        return Ok(0);
    }

    kasumi_mount::apply_runtime_config(&config)
        .context("failed to rebuild Kasumi runtime with user hide rules")?;
    Ok(rules.len())
}

pub fn apply_user_hide_rules_from_paths(rules: &[PathBuf]) -> Result<usize> {
    for path in rules {
        validate_hide_path(path)?;
        kasumi::hide_path(path)
            .with_context(|| format!("failed to apply hide rule for {}", path.display()))?;
    }

    Ok(rules.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_user_hide_rules_round_trips_paths() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("user_hide_rules.json");
        let rules = vec![
            PathBuf::from("/system/bin/example"),
            PathBuf::from("/vendor/lib64/libexample.so"),
        ];

        save_user_hide_rules_to(&path, &rules).unwrap();

        assert_eq!(load_user_hide_rules_from(&path).unwrap(), rules);
    }

    #[test]
    fn load_user_hide_rules_returns_empty_when_missing() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("missing.json");

        assert!(load_user_hide_rules_from(&path).unwrap().is_empty());
    }

    #[test]
    fn hide_paths_must_be_absolute_normalized_and_not_root() {
        assert!(validate_hide_path(Path::new("relative/path")).is_err());
        assert!(validate_hide_path(Path::new("/")).is_err());
        assert!(validate_hide_path(Path::new("/system/../data/file")).is_err());
        assert_eq!(
            validate_hide_path(Path::new("/data/adb/modules/example")).unwrap(),
            PathBuf::from("/data/adb/modules/example")
        );
    }

    #[test]
    fn hide_paths_reject_radio_and_firmware_targets() {
        assert!(validate_hide_path(Path::new("/vendor/firmware/modem.mbn")).is_err());
        assert!(validate_hide_path(Path::new("/system/vendor/etc/bluetooth/bt_vendor.conf")).is_err());
        assert!(validate_hide_path(Path::new("/vendor/lib64/libbt-vendor.so")).is_err());
        assert!(validate_hide_path(Path::new("/vendor/lib64/soundfx/libdolby.so")).is_ok());
    }

    #[test]
    fn removal_loader_accepts_legacy_critical_rules_but_strict_loader_rejects_them() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("user_hide_rules.json");
        fs::write(&path, r#"["/vendor/firmware/modem.mbn"]"#).unwrap();

        assert!(load_user_hide_rules_from(&path).is_err());
        assert_eq!(
            load_user_hide_rules_for_removal(&path).unwrap(),
            vec![PathBuf::from("/vendor/firmware/modem.mbn")]
        );
    }

    #[test]
    fn duplicate_rules_are_deduplicated_during_load() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("user_hide_rules.json");
        fs::write(
            &path,
            r#"[
  "/system/bin/example",
  "/system/bin/example"
]"#,
        )
        .unwrap();

        assert_eq!(
            load_user_hide_rules_from(&path).unwrap(),
            vec![PathBuf::from("/system/bin/example")]
        );
    }
}
