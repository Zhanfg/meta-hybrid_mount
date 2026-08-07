// Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-only

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use super::{
    cleanup,
    common::{
        build_managed_partitions, effective_maps_spoof_enabled, effective_mount_hide_enabled,
        effective_selinux_fix_enabled, effective_statfs_spoof_enabled, effective_stealth_enabled,
        feature_supported, has_uname_spoof_config, to_c_long, to_c_ulong,
    },
    compile::{CompiledRules, compile_rules, log_compiled_rule_summary, virtual_target_is_managed},
    status::{can_operate, hook_lines},
};
use crate::{
    conf::{
        config,
        schema::{self, KasumiUnameMode},
    },
    core::{inventory::Module, ops::plan::MountPlan, runtime_state::RuntimeState, user_hide_rules},
    defs,
    sys::{
        fs::atomic_write,
        kasumi::{
            self, KSM_FEATURE_CMDLINE_SPOOF, KSM_FEATURE_KSTAT_SPOOF, KSM_FEATURE_MAPS_SPOOF,
            KSM_FEATURE_MOUNT_HIDE, KSM_FEATURE_SELINUX_FIX, KSM_FEATURE_STATFS_SPOOF,
            KSM_FEATURE_UNAME_SPOOF, KasumiMapsRule, KasumiMountHideArg, KasumiSpoofKstat,
            KasumiSpoofUname, KasumiStatfsSpoofArg,
        },
        lkm,
    },
    utils,
};

const RULE_SNAPSHOT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct KasumiRuleSnapshot {
    schema_version: u32,
    module_ids: Vec<String>,
    mirror_path: PathBuf,
    rules: CompiledRules,
}

fn mount_mapping_requested(plan: &MountPlan) -> bool {
    !plan.kasumi_module_ids.is_empty()
}

fn auxiliary_features_requested(config: &config::Config) -> Result<bool> {
    Ok(config.kasumi.enable_kernel_debug
        || effective_stealth_enabled(config)
        || effective_mount_hide_enabled(config)
        || effective_maps_spoof_enabled(config)
        || effective_statfs_spoof_enabled(config)
        || effective_selinux_fix_enabled(config)
        || has_uname_spoof_config(config)
        || !config.kasumi.cmdline_value.is_empty()
        || !config.kasumi.hide_uids.is_empty()
        || !config.kasumi.kstat_rules.is_empty()
        || user_hide_rules::user_hide_rule_count()? > 0)
}

fn kasumi_runtime_requested(plan: &MountPlan, config: &config::Config) -> Result<bool> {
    Ok(config.kasumi.enabled
        && (mount_mapping_requested(plan) || auxiliary_features_requested(config)?))
}

fn apply_feature_toggle<F>(
    feature_name: &str,
    enabled: bool,
    features: i32,
    required_feature: i32,
    operation: F,
) -> Result<()>
where
    F: FnOnce(bool) -> Result<()>,
{
    let supported = feature_supported(features, required_feature);

    if !supported {
        bail!("Kasumi feature {feature_name} is not supported by the kernel module");
    }

    operation(enabled).with_context(|| format!("failed to apply Kasumi feature {feature_name}"))
}

fn get_features() -> Result<i32> {
    kasumi::get_features().context("failed to query Kasumi features")
}

fn log_feature_summary(features: i32) {
    let names = kasumi::feature_names(features);
    crate::scoped_log!(
        info,
        "mount:kasumi",
        "features: bits={}, names={}",
        features,
        if names.is_empty() {
            "none".to_string()
        } else {
            names.join(",")
        }
    );
}

fn require_feature(
    requested: bool,
    features: i32,
    required_feature: i32,
    feature_name: &str,
) -> Result<()> {
    if requested && !feature_supported(features, required_feature) {
        bail!("Kasumi feature {feature_name} is not supported by the kernel module");
    }
    Ok(())
}

fn validate_requested_feature_support(config: &config::Config, features: i32) -> Result<()> {
    require_feature(
        effective_mount_hide_enabled(config),
        features,
        KSM_FEATURE_MOUNT_HIDE,
        "mount_hide",
    )?;
    require_feature(
        effective_maps_spoof_enabled(config) || !config.kasumi.maps_rules.is_empty(),
        features,
        KSM_FEATURE_MAPS_SPOOF,
        "maps_spoof",
    )?;
    require_feature(
        effective_statfs_spoof_enabled(config),
        features,
        KSM_FEATURE_STATFS_SPOOF,
        "statfs_spoof",
    )?;
    require_feature(
        effective_selinux_fix_enabled(config),
        features,
        KSM_FEATURE_SELINUX_FIX,
        "selinux_fix",
    )?;
    require_feature(
        has_uname_spoof_config(config)
            || matches!(config.kasumi.uname_mode, KasumiUnameMode::Global),
        features,
        KSM_FEATURE_UNAME_SPOOF,
        "uname_spoof",
    )?;
    require_feature(
        !config.kasumi.cmdline_value.is_empty(),
        features,
        KSM_FEATURE_CMDLINE_SPOOF,
        "cmdline_spoof",
    )?;
    require_feature(
        !config.kasumi.kstat_rules.is_empty(),
        features,
        KSM_FEATURE_KSTAT_SPOOF,
        "kstat_spoof",
    )?;
    Ok(())
}

fn apply_runtime_switches(
    config: &config::Config,
    runtime_requested: bool,
    features: i32,
) -> Result<()> {
    if !runtime_requested {
        return Ok(());
    }

    if config.kasumi.enable_kernel_debug {
        kasumi::set_debug(true)?;
    }

    if effective_stealth_enabled(config) {
        kasumi::set_stealth(true)?;
    }

    if effective_mount_hide_enabled(config) {
        apply_mount_hide_from_config(config)?;
    }

    if effective_maps_spoof_enabled(config) {
        apply_feature_toggle(
            "maps_spoof",
            true,
            features,
            KSM_FEATURE_MAPS_SPOOF,
            kasumi::set_maps_spoof,
        )?;
    }

    if effective_statfs_spoof_enabled(config) {
        apply_statfs_spoof_from_config(config)?;
    }

    let selinux_fix_enabled = effective_selinux_fix_enabled(config);
    if feature_supported(features, KSM_FEATURE_SELINUX_FIX) {
        kasumi::set_selinux_fix(selinux_fix_enabled)?;
    }

    Ok(())
}

pub fn apply_mount_hide_from_config(config: &config::Config) -> Result<()> {
    let enabled = effective_mount_hide_enabled(config);

    if enabled && !config.kasumi.mount_hide.path_pattern.as_os_str().is_empty() {
        let arg =
            KasumiMountHideArg::new(true, Some(config.kasumi.mount_hide.path_pattern.as_path()))?;
        kasumi::set_mount_hide_config(&arg)
    } else {
        kasumi::set_mount_hide(enabled)
    }
}

pub fn apply_statfs_spoof_from_config(config: &config::Config) -> Result<()> {
    let enabled = effective_statfs_spoof_enabled(config);

    if enabled
        && (!config.kasumi.statfs_spoof.path.as_os_str().is_empty()
            || config.kasumi.statfs_spoof.spoof_f_type != 0)
    {
        let arg = KasumiStatfsSpoofArg::with_path_and_f_type(
            true,
            config.kasumi.statfs_spoof.path.as_path(),
            to_c_ulong(
                config.kasumi.statfs_spoof.spoof_f_type,
                "statfs_spoof.spoof_f_type",
            )?,
        )?;
        kasumi::set_statfs_spoof_config(&arg)
    } else {
        kasumi::set_statfs_spoof(enabled)
    }
}

pub fn apply_uname_from_config(config: &config::Config) -> Result<()> {
    let mut uname = KasumiSpoofUname::default();
    if !config.kasumi.uname.sysname.is_empty() {
        uname.set_sysname(&config.kasumi.uname.sysname)?;
    }
    if !config.kasumi.uname.nodename.is_empty() {
        uname.set_nodename(&config.kasumi.uname.nodename)?;
    }
    if !config.kasumi.uname.release.is_empty() {
        uname.set_release(&config.kasumi.uname.release)?;
    }
    if !config.kasumi.uname.version.is_empty() {
        uname.set_version(&config.kasumi.uname.version)?;
    }
    if !config.kasumi.uname.machine.is_empty() {
        uname.set_machine(&config.kasumi.uname.machine)?;
    }
    if !config.kasumi.uname.domainname.is_empty() {
        uname.set_domainname(&config.kasumi.uname.domainname)?;
    }
    match config.kasumi.uname_mode {
        KasumiUnameMode::Scoped => kasumi::set_uname(&uname),
        KasumiUnameMode::Global => kasumi::set_uname_global(&uname),
    }
}

pub fn apply_kstat_rule(rule: &schema::KasumiKstatRuleConfig) -> Result<()> {
    let mut native_rule = KasumiSpoofKstat::new(
        to_c_ulong(rule.target_ino, "target_ino")?,
        &rule.target_pathname,
    )?;
    native_rule.spoofed_ino = to_c_ulong(rule.spoofed_ino, "spoofed_ino")?;
    native_rule.spoofed_dev = to_c_ulong(rule.spoofed_dev, "spoofed_dev")?;
    native_rule.spoofed_nlink = rule.spoofed_nlink;
    native_rule.spoofed_size = rule.spoofed_size;
    native_rule.spoofed_atime_sec = to_c_long(rule.spoofed_atime_sec, "spoofed_atime_sec")?;
    native_rule.spoofed_atime_nsec = to_c_long(rule.spoofed_atime_nsec, "spoofed_atime_nsec")?;
    native_rule.spoofed_mtime_sec = to_c_long(rule.spoofed_mtime_sec, "spoofed_mtime_sec")?;
    native_rule.spoofed_mtime_nsec = to_c_long(rule.spoofed_mtime_nsec, "spoofed_mtime_nsec")?;
    native_rule.spoofed_ctime_sec = to_c_long(rule.spoofed_ctime_sec, "spoofed_ctime_sec")?;
    native_rule.spoofed_ctime_nsec = to_c_long(rule.spoofed_ctime_nsec, "spoofed_ctime_nsec")?;
    native_rule.spoofed_blksize = to_c_ulong(rule.spoofed_blksize, "spoofed_blksize")?;
    native_rule.spoofed_blocks = rule.spoofed_blocks;
    native_rule.is_static = if rule.is_static { 1 } else { 0 };

    kasumi::add_spoof_kstat(&native_rule).with_context(|| {
        format!(
            "failed to apply kstat rule for {}",
            rule.target_pathname.display()
        )
    })
}

fn apply_spoof_settings(config: &config::Config, features: i32) -> Result<()> {
    let has_uname_config = has_uname_spoof_config(config);
    let should_apply_uname =
        has_uname_config || matches!(config.kasumi.uname_mode, KasumiUnameMode::Global);
    if should_apply_uname {
        apply_uname_from_config(config)?;
    }

    if !config.kasumi.cmdline_value.is_empty() {
        kasumi::set_cmdline_str(&config.kasumi.cmdline_value)?;
    }

    if !config.kasumi.hide_uids.is_empty() {
        kasumi::set_hide_uids(&config.kasumi.hide_uids)?;
    }

    if !config.kasumi.kstat_rules.is_empty() {
        for rule in &config.kasumi.kstat_rules {
            apply_kstat_rule(rule)?;
        }
    }

    if !config.kasumi.maps_rules.is_empty() {
        for rule in &config.kasumi.maps_rules {
            let native_rule = KasumiMapsRule::new(
                to_c_ulong(rule.target_ino, "target_ino")?,
                to_c_ulong(rule.target_dev, "target_dev")?,
                to_c_ulong(rule.spoofed_ino, "spoofed_ino")?,
                to_c_ulong(rule.spoofed_dev, "spoofed_dev")?,
                &rule.spoofed_pathname,
            )?;
            kasumi::add_maps_rule(&native_rule)?;
        }
    }

    Ok(())
}

pub fn reset_runtime(config: &config::Config) -> Result<bool> {
    if !config.kasumi.enabled {
        return Ok(false);
    }

    let available = can_operate(config)?;
    if !available {
        bail!("Kasumi is enabled but unavailable during runtime reset");
    }

    crate::scoped_log!(
        info,
        "mount:kasumi",
        "reset: mirror_path={}",
        config.kasumi.mirror_path.display()
    );

    cleanup::rollback_runtime().context("failed to reset complete Kasumi runtime state")?;
    kasumi::set_mirror_path(&config.kasumi.mirror_path)?;

    let features = get_features()?;
    log_feature_summary(features);

    if config.kasumi.mirror_path != Path::new(defs::KASUMI_MIRROR_DIR) {
        crate::scoped_log!(
            info,
            "mount:kasumi",
            "custom mirror active: path={}",
            config.kasumi.mirror_path.display()
        );
    }

    Ok(true)
}

fn normalize_module_ids(mut module_ids: Vec<String>) -> Vec<String> {
    module_ids.sort();
    module_ids.dedup();
    module_ids
}

fn compiled_rules_from_plan(plan: &MountPlan) -> CompiledRules {
    CompiledRules {
        add_rules: plan.kasumi_add_rules.clone(),
        merge_rules: plan.kasumi_merge_rules.clone(),
        hide_rules: plan.kasumi_hide_rules.clone(),
    }
}

fn save_rule_snapshot(plan: &MountPlan, config: &config::Config) -> Result<()> {
    let snapshot = KasumiRuleSnapshot {
        schema_version: RULE_SNAPSHOT_SCHEMA_VERSION,
        module_ids: normalize_module_ids(plan.kasumi_module_ids.clone()),
        mirror_path: utils::normalize_path(&config.kasumi.mirror_path),
        rules: compiled_rules_from_plan(plan),
    };
    let payload = serde_json::to_vec_pretty(&snapshot)
        .context("failed to serialize Kasumi module rule snapshot")?;
    atomic_write(defs::KASUMI_RULE_SNAPSHOT_FILE, payload).with_context(|| {
        format!(
            "failed to persist Kasumi module rule snapshot {}",
            defs::KASUMI_RULE_SNAPSHOT_FILE
        )
    })
}

fn load_rule_snapshot() -> Result<KasumiRuleSnapshot> {
    let payload = fs::read(defs::KASUMI_RULE_SNAPSHOT_FILE).with_context(|| {
        format!(
            "failed to read Kasumi module rule snapshot {}; reboot is required before live configuration changes",
            defs::KASUMI_RULE_SNAPSHOT_FILE
        )
    })?;
    let snapshot: KasumiRuleSnapshot = serde_json::from_slice(&payload)
        .context("failed to parse Kasumi module rule snapshot; reboot is required")?;
    if snapshot.schema_version != RULE_SNAPSHOT_SCHEMA_VERSION {
        bail!(
            "unsupported Kasumi rule snapshot schema {}; expected {}; reboot is required",
            snapshot.schema_version,
            RULE_SNAPSHOT_SCHEMA_VERSION
        );
    }
    Ok(snapshot)
}

fn validate_rule_target(target: &str, managed_partitions: &HashSet<String>) -> Result<()> {
    let path = Path::new(target);
    if !path.is_absolute() || utils::normalize_path(path) != path {
        bail!("invalid persisted Kasumi target path: {target}");
    }
    if !virtual_target_is_managed(path, managed_partitions) {
        bail!("persisted Kasumi target escaped managed partitions: {target}");
    }
    Ok(())
}

fn validate_add_source(source: &Path, mirror_root: &Path) -> Result<()> {
    if !source.is_absolute() || !source.starts_with(mirror_root) {
        bail!(
            "persisted Kasumi ADD source escaped mirror root: source={}, mirror={}",
            source.display(),
            mirror_root.display()
        );
    }
    fs::symlink_metadata(source).with_context(|| {
        format!(
            "persisted Kasumi ADD source is unavailable: {}",
            source.display()
        )
    })?;
    let parent = source
        .parent()
        .context("persisted Kasumi ADD source has no parent")?;
    let resolved_parent = fs::canonicalize(parent).with_context(|| {
        format!(
            "failed to resolve persisted Kasumi ADD source parent {}",
            parent.display()
        )
    })?;
    if !resolved_parent.starts_with(mirror_root) {
        bail!(
            "persisted Kasumi ADD source parent escaped mirror root: source={}, resolved_parent={}, mirror={}",
            source.display(),
            resolved_parent.display(),
            mirror_root.display()
        );
    }
    Ok(())
}

fn validate_merge_source(source: &Path, mirror_root: &Path) -> Result<()> {
    let resolved = fs::canonicalize(source).with_context(|| {
        format!(
            "failed to resolve persisted Kasumi MERGE source {}",
            source.display()
        )
    })?;
    if !resolved.is_dir() || !resolved.starts_with(mirror_root) {
        bail!(
            "persisted Kasumi MERGE source escaped mirror root or is not a directory: source={}, resolved={}, mirror={}",
            source.display(),
            resolved.display(),
            mirror_root.display()
        );
    }
    Ok(())
}

fn validate_file_type(file_type: i32) -> Result<()> {
    let supported = [
        libc::DT_UNKNOWN as i32,
        libc::DT_REG as i32,
        libc::DT_LNK as i32,
        libc::DT_BLK as i32,
        libc::DT_CHR as i32,
        libc::DT_FIFO as i32,
        libc::DT_SOCK as i32,
    ];
    if !supported.contains(&file_type) {
        bail!("invalid persisted Kasumi file type: {file_type}");
    }
    Ok(())
}

fn validate_rule_snapshot(
    snapshot: &KasumiRuleSnapshot,
    active_module_ids: &[String],
    config: &config::Config,
) -> Result<()> {
    if normalize_module_ids(snapshot.module_ids.clone()) != active_module_ids {
        bail!(
            "Kasumi rule snapshot module set does not match the current boot; reboot is required"
        );
    }

    let configured_mirror = utils::normalize_path(&config.kasumi.mirror_path);
    if snapshot.mirror_path != configured_mirror {
        bail!(
            "changing Kasumi mirror_path with active module rules requires a reboot: active={}, requested={}",
            snapshot.mirror_path.display(),
            configured_mirror.display()
        );
    }

    let mirror_root = fs::canonicalize(&config.kasumi.mirror_path).with_context(|| {
        format!(
            "failed to resolve Kasumi mirror root {}",
            config.kasumi.mirror_path.display()
        )
    })?;
    let managed_partitions = build_managed_partitions(config);

    for rule in &snapshot.rules.add_rules {
        validate_rule_target(&rule.target, &managed_partitions)?;
        validate_file_type(rule.file_type)?;
        validate_add_source(&rule.source, &mirror_root)?;
    }
    for rule in &snapshot.rules.merge_rules {
        validate_rule_target(&rule.target, &managed_partitions)?;
        validate_merge_source(&rule.source, &mirror_root)?;
    }
    for target in &snapshot.rules.hide_rules {
        validate_rule_target(target, &managed_partitions)?;
    }

    Ok(())
}

fn build_live_runtime_plan(config: &config::Config) -> Result<(MountPlan, CompiledRules)> {
    let state = RuntimeState::load().context("failed to load current mount runtime state")?;
    let active_module_ids = normalize_module_ids(state.kasumi_modules);

    if active_module_ids.is_empty() {
        return Ok((MountPlan::default(), CompiledRules::default()));
    }

    let snapshot = load_rule_snapshot()?;
    validate_rule_snapshot(&snapshot, &active_module_ids, config)?;

    Ok((
        MountPlan {
            kasumi_module_ids: active_module_ids,
            ..MountPlan::default()
        },
        snapshot.rules,
    ))
}

fn preflight_live_runtime(
    snapshot_rules: &CompiledRules,
    plan: &MountPlan,
    config: &config::Config,
) -> Result<()> {
    if mount_mapping_requested(plan) {
        let snapshot = KasumiRuleSnapshot {
            schema_version: RULE_SNAPSHOT_SCHEMA_VERSION,
            module_ids: plan.kasumi_module_ids.clone(),
            mirror_path: utils::normalize_path(&config.kasumi.mirror_path),
            rules: snapshot_rules.clone(),
        };
        validate_rule_snapshot(&snapshot, &plan.kasumi_module_ids, config)?;
    }
    user_hide_rules::load_user_hide_rules()
        .context("failed to validate user hide rules before live rebuild")?;
    let features = get_features()?;
    validate_requested_feature_support(config, features)
}

pub fn apply_runtime_config(config: &config::Config) -> Result<bool> {
    if config.kasumi.enabled && !can_operate(config)? {
        lkm::autoload_if_needed(&config.kasumi)?;
    }

    if !can_operate(config)? {
        if config.kasumi.enabled {
            bail!("Kasumi is enabled but unavailable");
        }
        return Ok(false);
    }

    let (mut plan, snapshot_rules) = build_live_runtime_plan(config)?;
    preflight_live_runtime(&snapshot_rules, &plan, config)?;
    reset_runtime(config)?;
    apply_compiled(&mut plan, snapshot_rules, config)
}

pub fn apply(plan: &mut MountPlan, modules: &[Module], config: &config::Config) -> Result<bool> {
    if !config.kasumi.enabled {
        return Ok(false);
    }

    let compiled = if mount_mapping_requested(plan) {
        compile_rules(modules, plan, config)?
    } else {
        CompiledRules::default()
    };
    apply_compiled(plan, compiled, config)
}

fn apply_compiled(
    plan: &mut MountPlan,
    compiled: CompiledRules,
    config: &config::Config,
) -> Result<bool> {
    if !config.kasumi.enabled {
        return Ok(false);
    }

    let runtime_requested = kasumi_runtime_requested(plan, config)?;
    if !can_operate(config)? {
        bail!("Kasumi became unavailable before rule application");
    }

    crate::scoped_log!(
        info,
        "mount:kasumi",
        "apply: mirror_path={}, kasumi_modules={}, runtime_requested={}",
        config.kasumi.mirror_path.display(),
        plan.kasumi_module_ids.len(),
        runtime_requested
    );

    let user_hide_paths = user_hide_rules::load_user_hide_rules()?;
    log_compiled_rule_summary(&compiled, &user_hide_paths);

    plan.kasumi_add_rules = compiled.add_rules;
    plan.kasumi_merge_rules = compiled.merge_rules;
    plan.kasumi_hide_rules = compiled.hide_rules;

    kasumi::set_mirror_path(&config.kasumi.mirror_path)?;
    kasumi::clear_rules()?;
    kasumi::clear_maps_rules()?;

    let features = get_features()?;
    log_feature_summary(features);
    validate_requested_feature_support(config, features)?;

    if !runtime_requested {
        kasumi::set_enabled(false)?;
        save_rule_snapshot(plan, config)?;
        crate::scoped_log!(
            info,
            "mount:kasumi",
            "apply skipped: reason=no_runtime_request"
        );
        return Ok(false);
    }

    apply_runtime_switches(config, true, features)?;
    apply_spoof_settings(config, features)?;

    for rule in &plan.kasumi_add_rules {
        kasumi::add_rule(Path::new(&rule.target), &rule.source, rule.file_type)?;
    }
    for rule in &plan.kasumi_merge_rules {
        kasumi::add_merge_rule(Path::new(&rule.target), &rule.source)?;
    }
    for path in &plan.kasumi_hide_rules {
        kasumi::hide_path(Path::new(path))?;
    }

    let user_hide_applied = user_hide_rules::apply_user_hide_rules_from_paths(&user_hide_paths)?;

    kasumi::set_enabled(true)?;
    kasumi::fix_mounts()?;
    save_rule_snapshot(plan, config)?;

    crate::scoped_log!(
        info,
        "mount:kasumi",
        "apply complete: enabled=true, add_rules={}, merge_rules={}, hide_rules={}, maps_rules={}, kstat_rules={}",
        plan.kasumi_add_rules.len(),
        plan.kasumi_merge_rules.len(),
        plan.kasumi_hide_rules.len(),
        config.kasumi.maps_rules.len(),
        config.kasumi.kstat_rules.len()
    );

    if user_hide_applied > 0 {
        crate::scoped_log!(
            info,
            "mount:kasumi",
            "user hide rules: applied={}",
            user_hide_applied
        );
    }

    let hooks = hook_lines()?;
    crate::scoped_log!(debug, "mount:kasumi", "hooks: {}", hooks.join(","));

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_ids_are_sorted_and_deduplicated() {
        assert_eq!(
            normalize_module_ids(vec!["b".to_string(), "a".to_string(), "b".to_string()]),
            vec!["a", "b"]
        );
    }

    #[test]
    fn persisted_targets_must_be_absolute_normalized_and_managed() {
        let managed = HashSet::from(["system".to_string(), "vendor".to_string()]);

        assert!(validate_rule_target("/system/etc/file", &managed).is_ok());
        assert!(validate_rule_target("/data/local/tmp/file", &managed).is_err());
        assert!(validate_rule_target("system/etc/file", &managed).is_err());
        assert!(validate_rule_target("/system/../data/file", &managed).is_err());
    }

    #[test]
    fn persisted_file_types_are_restricted_to_kernel_dirent_values() {
        assert!(validate_file_type(libc::DT_REG as i32).is_ok());
        assert!(validate_file_type(libc::DT_LNK as i32).is_ok());
        assert!(validate_file_type(9999).is_err());
    }
}
