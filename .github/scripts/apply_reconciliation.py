from pathlib import Path


def replace_once(path: Path, old: str, new: str) -> None:
    text = path.read_text()
    count = text.count(old)
    if count != 1:
        raise SystemExit(
            f"{path}: expected one match, found {count}: {old[:140]!r}"
        )
    path.write_text(text.replace(old, new, 1))


prepare = Path("src/core/ops/prepare/coordinator.rs")
replace_once(
    prepare,
    '''        #[cfg(feature = "kasumi")]
        kasumi_module_ids: sorted_ids(kasumi_ids),
    };
''',
    '''        #[cfg(feature = "kasumi")]
        kasumi_module_ids: sorted_ids(kasumi_ids),
        #[cfg(feature = "kasumi")]
        kasumi_fallback_module_ids: Vec::new(),
    };
''',
)

runtime = Path("src/mount/kasumi/runtime.rs")
replace_once(
    runtime,
    '''use super::{
    common::{
''',
    '''use super::{
    cleanup,
    common::{
''',
)
replace_once(
    runtime,
    '''    kasumi::set_mirror_path(&config.kasumi.mirror_path)?;
    kasumi::set_enabled(false)?;
    kasumi::clear_rules()?;
    kasumi::clear_maps_rules()?;

    let features = get_features()?;
''',
    '''    cleanup::rollback_runtime().context("failed to reset complete Kasumi runtime state")?;
    kasumi::set_mirror_path(&config.kasumi.mirror_path)?;

    let features = get_features()?;
''',
)

executor = Path("src/core/ops/executor/mod.rs")
replace_once(
    executor,
    '''        if let Err(error) = crate::sys::kasumi::set_enabled(false) {
            crate::scoped_log!(
                error,
                "executor",
                "Kasumi rollback disable failed: error={:#}",
                error
            );
        }
        if let Err(error) = crate::sys::kasumi::clear_rules() {
            crate::scoped_log!(
                error,
                "executor",
                "Kasumi rollback rule cleanup failed: error={:#}",
                error
            );
        }
        if let Err(error) = crate::sys::kasumi::clear_maps_rules() {
            crate::scoped_log!(
                error,
                "executor",
                "Kasumi rollback maps cleanup failed: error={:#}",
                error
            );
        }
''',
    '''        match crate::mount::kasumi::rollback_runtime() {
            Ok(()) => crate::scoped_log!(
                warn,
                "executor",
                "Kasumi runtime rolled back after execution failure"
            ),
            Err(error) => crate::scoped_log!(
                error,
                "executor",
                "Kasumi runtime rollback incomplete: error={:#}",
                error
            ),
        }
''',
)
replace_once(
    executor,
    '''            "start: overlay_ops={}, preselected_magic_modules={}, preselected_kasumi_modules={}",
            plan.overlay_ops.len(),
            plan.magic_module_ids.len(),
            plan.kasumi_count()
''',
    '''            "start: overlay_ops={}, preselected_magic_modules={}, preselected_kasumi_modules={}, kasumi_fallback_modules={}",
            plan.overlay_ops.len(),
            plan.magic_module_ids.len(),
            plan.kasumi_count(),
            plan.kasumi_fallback_ids().len()
''',
)
replace_once(
    executor,
    '''        #[cfg(feature = "kasumi")]
        let final_kasumi_ids = plan.kasumi_module_ids.clone();
''',
    '''        #[cfg(feature = "kasumi")]
        let mut kasumi_guard = KasumiRuntimeGuard::new(kasumi_available);
        #[cfg(feature = "kasumi")]
        let final_kasumi_ids = plan.kasumi_module_ids.clone();
''',
)
replace_once(
    executor,
    '''        #[cfg(feature = "kasumi")]
        let mut kasumi_guard = KasumiRuntimeGuard::new(kasumi_runtime_enabled);
        #[cfg(not(feature = "kasumi"))]
''',
    '''        #[cfg(not(feature = "kasumi"))]
''',
)
replace_once(
    executor,
    '''                "magic apply: modules={}",
                magic_need_list.join(", ")
            );
            let (mounted_ids, magic_stats) =
                magic::mount_magic(modules, &magic_need_list, config, tempdir.as_ref()).map_err(
                    |err| {
                        ModuleStageFailure::new(
                            FailureStage::Execute,
                            magic_need_list.clone(),
                            anyhow::anyhow!(
                                "Failed to mount Magic Mount modules [{}]: {:#}",
                                magic_need_list.join(", "),
                                err
                            ),
                        )
                    },
                )?;
''',
    '''                "magic apply: modules={}, kasumi_fallback_modules={}",
                magic_need_list.join(", "),
                plan.kasumi_fallback_ids().len()
            );
            let (mounted_ids, magic_stats) = magic::mount_magic(
                modules,
                &magic_need_list,
                plan.kasumi_fallback_ids(),
                config,
                tempdir.as_ref(),
            )
            .map_err(|err| {
                ModuleStageFailure::new(
                    FailureStage::Execute,
                    magic_need_list.clone(),
                    anyhow::anyhow!(
                        "Failed to mount Magic Mount modules [{}]: {:#}",
                        magic_need_list.join(", "),
                        err
                    ),
                )
            })?;
''',
)

api = Path("src/core/api/kasumi.rs")
replace_once(
    api,
    '''pub struct LkmPayload {
    pub loaded: bool,
    pub module_name: Option<String>,
''',
    '''pub struct LkmPayload {
    pub loaded: bool,
    pub managed: bool,
    pub module_name: Option<String>,
''',
)
replace_once(
    api,
    '''        Self {
            loaded: status.loaded,
            module_name: status.module_name,
''',
    '''        Self {
            loaded: status.loaded,
            managed: status.managed,
            module_name: status.module_name,
''',
)
replace_once(
    api,
    '''#[cfg(test)]
mod tests {
    use super::parse_kasumi_rule_listing;
''',
    '''#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{LkmPayload, parse_kasumi_rule_listing};
    use crate::sys::lkm::LkmStatus;
''',
)
replace_once(
    api,
    '''    fn rule_listing_keeps_lowercase_path_rules_named_like_status_values() {
        let rules = parse_kasumi_rule_listing("hide enabled\\ninject disabled\\n").unwrap();

        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].path.as_deref(), Some("enabled"));
        assert_eq!(rules[1].path.as_deref(), Some("disabled"));
    }
''',
    '''    fn rule_listing_keeps_lowercase_path_rules_named_like_status_values() {
        let rules = parse_kasumi_rule_listing("hide enabled\\ninject disabled\\n").unwrap();

        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].path.as_deref(), Some("enabled"));
        assert_eq!(rules[1].path.as_deref(), Some("disabled"));
    }

    #[test]
    fn lkm_payload_preserves_management_ownership() {
        let payload = LkmPayload::from(LkmStatus {
            loaded: true,
            managed: false,
            module_name: Some("kasumi".to_string()),
            autoload: false,
            kmi_override: String::new(),
            current_kmi: "android15-6.6".to_string(),
            search_dir: PathBuf::from("/data/adb/modules/hybrid_mount/kasumi_lkm"),
            module_file: PathBuf::new(),
        });

        assert!(payload.loaded);
        assert!(!payload.managed);
        assert_eq!(payload.module_name.as_deref(), Some("kasumi"));
    }
''',
)

abi = Path("tests/kasumi_uapi_compat.c")
replace_once(
    abi,
    '''ASSERT_SIZE(struct kasumi_spoof_uname, 396);
ASSERT_OFFSET(struct kasumi_spoof_uname, err, 392);
''',
    '''ASSERT_SIZE(struct kasumi_spoof_uname, 396);
ASSERT_OFFSET(struct kasumi_spoof_uname, sysname, 0);
ASSERT_OFFSET(struct kasumi_spoof_uname, nodename, 65);
ASSERT_OFFSET(struct kasumi_spoof_uname, release, 130);
ASSERT_OFFSET(struct kasumi_spoof_uname, version, 195);
ASSERT_OFFSET(struct kasumi_spoof_uname, machine, 260);
ASSERT_OFFSET(struct kasumi_spoof_uname, domainname, 325);
/* Six 65-byte arrays occupy 390 bytes; the trailing int is 4-byte aligned. */
ASSERT_OFFSET(struct kasumi_spoof_uname, err, 392);
''',
)
