from pathlib import Path


def replace_once(path: Path, old: str, new: str) -> None:
    text = path.read_text()
    count = text.count(old)
    if count != 1:
        raise SystemExit(
            f"{path}: expected one match, found {count}: {old[:120]!r}"
        )
    path.write_text(text.replace(old, new, 1))


plan = Path("src/core/ops/plan.rs")
replace_once(
    plan,
    '''impl MountPlan {
    pub fn kasumi_count(&self) -> usize {
        #[cfg(feature = "kasumi")]
        {
            self.kasumi_module_ids.len()
        }
        #[cfg(not(feature = "kasumi"))]
        {
            0
        }
    }
}
''',
    '''impl MountPlan {
    pub fn kasumi_count(&self) -> usize {
        #[cfg(feature = "kasumi")]
        {
            self.kasumi_module_ids.len()
        }
        #[cfg(not(feature = "kasumi"))]
        {
            0
        }
    }

    #[cfg(feature = "kasumi")]
    pub fn degrade_kasumi_to_magic(&mut self) -> usize {
        let mut downgraded = std::mem::take(&mut self.kasumi_module_ids);
        let changed = downgraded.len();
        self.magic_module_ids.append(&mut downgraded);
        self.magic_module_ids.sort();
        self.magic_module_ids.dedup();
        self.kasumi_add_rules.clear();
        self.kasumi_merge_rules.clear();
        self.kasumi_hide_rules.clear();
        changed
    }
}

#[cfg(all(test, feature = "kasumi"))]
mod tests {
    use super::{KasumiAddRule, KasumiMergeRule, MountPlan};
    use std::path::PathBuf;

    #[test]
    fn kasumi_downgrade_preserves_module_coverage_and_clears_rules() {
        let mut plan = MountPlan {
            magic_module_ids: vec!["existing".to_string()],
            kasumi_module_ids: vec!["b".to_string(), "a".to_string(), "existing".to_string()],
            kasumi_add_rules: vec![KasumiAddRule {
                target: "/system/a".to_string(),
                source: PathBuf::from("/dev/a"),
                file_type: 1,
            }],
            kasumi_merge_rules: vec![KasumiMergeRule {
                target: "/system/b".to_string(),
                source: PathBuf::from("/dev/b"),
            }],
            kasumi_hide_rules: vec!["/system/c".to_string()],
            ..MountPlan::default()
        };

        assert_eq!(plan.degrade_kasumi_to_magic(), 3);
        assert_eq!(plan.magic_module_ids, vec!["a", "b", "existing"]);
        assert!(plan.kasumi_module_ids.is_empty());
        assert!(plan.kasumi_add_rules.is_empty());
        assert!(plan.kasumi_merge_rules.is_empty());
        assert!(plan.kasumi_hide_rules.is_empty());
    }
}
''',
)

storage = Path("src/core/storage/mod.rs")
replace_once(
    storage,
    '''pub fn cleanup_artifacts(storage_mode: StorageMode) -> Result<()> {
    if should_cleanup_image(storage_mode) {
        remove_image_file(Path::new(defs::MODULES_IMG_FILE))?;
    }

    Ok(())
}
''',
    '''pub fn cleanup_artifacts(storage_mode: StorageMode) -> Result<()> {
    if should_cleanup_image(storage_mode) {
        remove_image_file(Path::new(defs::MODULES_IMG_FILE))?;
    }

    Ok(())
}

pub fn cleanup_failed_setup(mount_point: &Path, image_path: &Path) -> Result<()> {
    detach_existing_mount(mount_point)?;
    reset_image_files(image_path)
}
''',
)

coordinator = Path("src/core/kasumi_coordinator.rs")
replace_once(coordinator, "use anyhow::{Result, bail};", "use anyhow::{Context, Result, bail};")
replace_once(
    coordinator,
    '''        let kasumi_storage = storage::setup_with_sources(
            &mirror_path,
            &kasumi_sources,
            matches!(self.config.overlay_mode, OverlayMode::Ext4),
            &self.config.mountsource,
            true,
            Path::new(defs::KASUMI_IMG_FILE),
        )?;

        mirror_sync::sync_modules(&kasumi_modules, kasumi_storage.mount_point())?;
''',
    '''        let image_path = Path::new(defs::KASUMI_IMG_FILE);
        let kasumi_storage = match storage::setup_with_sources(
            &mirror_path,
            &kasumi_sources,
            matches!(self.config.overlay_mode, OverlayMode::Ext4),
            &self.config.mountsource,
            true,
            image_path,
        ) {
            Ok(handle) => handle,
            Err(error) => {
                if let Err(cleanup_error) = storage::cleanup_failed_setup(&mirror_path, image_path) {
                    bail!(
                        "Kasumi mirror setup failed and cleanup also failed: setup={:#}; cleanup={:#}",
                        error,
                        cleanup_error
                    );
                }
                return Err(error).context("failed to set up Kasumi mirror storage");
            }
        };

        if let Err(error) = mirror_sync::sync_modules(&kasumi_modules, kasumi_storage.mount_point()) {
            if let Err(cleanup_error) = storage::cleanup_failed_setup(&mirror_path, image_path) {
                bail!(
                    "Kasumi mirror synchronization failed and cleanup also failed: sync={:#}; cleanup={:#}",
                    error,
                    cleanup_error
                );
            }
            return Err(error).context("failed to synchronize Kasumi mirror modules");
        }
''',
)

controller = Path("src/core/controller.rs")
replace_once(
    controller,
    '''#[cfg(feature = "kasumi")]
use crate::core::failure::ModuleStageFailure;
''',
    "",
)
replace_once(controller, "        let plan = prepare::prepare_mount_plan(\n", "        let mut plan = prepare::prepare_mount_plan(\n")
replace_once(
    controller,
    '''            kasumi
                .prepare_mirror_storage(
                    &self.backend_capabilities,
                    modules,
                    &plan,
                    self.state.handle.mount_point(),
                )
                .map_err(|err| {
                    ModuleStageFailure::sync(
                        plan.kasumi_module_ids.clone(),
                        anyhow::anyhow!("Failed to prepare Kasumi mirror storage: {:#}", err),
                    )
                })?;
''',
    '''            if let Err(error) = kasumi.prepare_mirror_storage(
                &self.backend_capabilities,
                modules,
                &plan,
                self.state.handle.mount_point(),
            ) {
                let changed = plan.degrade_kasumi_to_magic();
                crate::scoped_log!(
                    warn,
                    "controller:scan_and_prepare_plan",
                    "Kasumi mirror preparation failed; downgraded current boot to Magic Mount: modules={}, persistent_config_unchanged=true, error={:#}",
                    changed,
                    error
                );
            }
''',
)

executor = Path("src/core/ops/executor/mod.rs")
replace_once(
    executor,
    '''}

pub struct Executor;
''',
    '''}

#[cfg(feature = "kasumi")]
struct KasumiRuntimeGuard {
    armed: bool,
}

#[cfg(feature = "kasumi")]
impl KasumiRuntimeGuard {
    fn new(armed: bool) -> Self {
        Self { armed }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

#[cfg(feature = "kasumi")]
impl Drop for KasumiRuntimeGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if let Err(error) = crate::sys::kasumi::set_enabled(false) {
            crate::scoped_log!(error, "executor", "Kasumi rollback disable failed: error={:#}", error);
        }
        if let Err(error) = crate::sys::kasumi::clear_rules() {
            crate::scoped_log!(error, "executor", "Kasumi rollback rule cleanup failed: error={:#}", error);
        }
        if let Err(error) = crate::sys::kasumi::clear_maps_rules() {
            crate::scoped_log!(error, "executor", "Kasumi rollback maps cleanup failed: error={:#}", error);
        }
    }
}

pub struct Executor;
''',
)
replace_once(
    executor,
    '''        #[cfg(feature = "kasumi")]
        if !kasumi_available && !planned_kasumi_ids.is_empty() {
            return Err(ModuleStageFailure::new(
                FailureStage::Execute,
                planned_kasumi_ids.clone(),
                anyhow::anyhow!("Kasumi became unavailable before execution"),
            )
            .into());
        }

        if Self::is_supported()? {
''',
    '''        #[cfg(feature = "kasumi")]
        if !kasumi_available && !planned_kasumi_ids.is_empty() {
            return Err(ModuleStageFailure::new(
                FailureStage::Execute,
                planned_kasumi_ids.clone(),
                anyhow::anyhow!("Kasumi became unavailable before execution"),
            )
            .into());
        }

        #[cfg(feature = "kasumi")]
        let final_kasumi_ids = plan.kasumi_module_ids.clone();
        #[cfg(feature = "kasumi")]
        let kasumi_runtime_enabled = if config.kasumi.enabled {
            kasumi.apply_runtime(plan, modules).map_err(|error| {
                ModuleStageFailure::new(
                    FailureStage::Execute,
                    final_kasumi_ids.clone(),
                    anyhow::anyhow!(
                        "Failed to apply Kasumi before other mount backends: {:#}",
                        error
                    ),
                )
            })?
        } else {
            crate::scoped_log!(debug, "executor", "kasumi disabled: skip_runtime_apply=true");
            false
        };
        #[cfg(feature = "kasumi")]
        let mut kasumi_guard = KasumiRuntimeGuard::new(kasumi_runtime_enabled);
        #[cfg(not(feature = "kasumi"))]
        let kasumi_runtime_enabled = false;

        if Self::is_supported()? {
''',
)
replace_once(
    executor,
    '''        #[cfg(feature = "kasumi")]
        {
            plan.kasumi_add_rules.clear();
            plan.kasumi_merge_rules.clear();
            plan.kasumi_hide_rules.clear();
        }
        #[cfg(feature = "kasumi")]
        let final_kasumi_ids = plan.kasumi_module_ids.clone();

''',
    "",
)
replace_once(
    executor,
    '''        #[cfg(feature = "kasumi")]
        let kasumi_runtime_enabled = if config.kasumi.enabled {
            kasumi.apply_runtime(plan, modules).map_err(|err| {
                ModuleStageFailure::new(
                    FailureStage::Execute,
                    final_kasumi_ids.clone(),
                    anyhow::anyhow!("Failed to apply Kasumi late rules: {:#}", err),
                )
            })?
        } else {
            crate::scoped_log!(
                debug,
                "executor",
                "kasumi disabled: skip_runtime_apply=true"
            );
            false
        };
        #[cfg(not(feature = "kasumi"))]
        let kasumi_runtime_enabled = false;

''',
    '''        #[cfg(feature = "kasumi")]
        if kasumi_runtime_enabled {
            if let Err(error) = crate::sys::kasumi::fix_mounts() {
                crate::scoped_log!(
                    warn,
                    "executor",
                    "Kasumi mount-id refresh failed after other backends: error={:#}",
                    error
                );
            }
        }

''',
)
replace_once(
    executor,
    '''        #[cfg(any(target_os = "linux", target_os = "android"))]
        if !config.disable_umount {
            umount_mgr::commit().context("Failed to commit umountable mount list")?;
        }
''',
    '''        #[cfg(any(target_os = "linux", target_os = "android"))]
        if !config.disable_umount {
            if let Err(error) = umount_mgr::commit() {
                crate::scoped_log!(
                    warn,
                    "executor",
                    "umountable mount-list commit failed after successful mounts: error={:#}",
                    error
                );
            }
        }
''',
)
replace_once(
    executor,
    '''        crate::scoped_log!(
            info,
            "executor",
            "complete: overlay_modules={}, magic_modules={}, custom_mounts={}, kasumi_modules={}",
            result_overlay.len(),
            result_magic.len(),
            custom_mount_targets.len(),
            kasumi_count
        );

        Ok(ExecutionResult {
''',
    '''        crate::scoped_log!(
            info,
            "executor",
            "complete: overlay_modules={}, magic_modules={}, custom_mounts={}, kasumi_modules={}",
            result_overlay.len(),
            result_magic.len(),
            custom_mount_targets.len(),
            kasumi_count
        );

        #[cfg(feature = "kasumi")]
        kasumi_guard.disarm();

        Ok(ExecutionResult {
''',
)

lkm = Path("src/sys/lkm.rs")
replace_once(
    lkm,
    '''pub struct LkmStatus {
    pub loaded: bool,
    pub module_name: Option<String>,
''',
    '''pub struct LkmStatus {
    pub loaded: bool,
    pub managed: bool,
    pub module_name: Option<String>,
''',
)
replace_once(
    lkm,
    '''pub fn status(config: &KasumiConfig) -> Result<LkmStatus> {
    let module_name = loaded_module_name()?;
    Ok(LkmStatus {
        loaded: module_name.is_some(),
        module_name,
        autoload: config.lkm_autoload,
        kmi_override: config.lkm_kmi_override.clone(),
        current_kmi: current_kmi()?,
        search_dir: config.lkm_dir.clone(),
        module_file: resolve_module_file(config)?,
    })
}
''',
    '''pub fn status(config: &KasumiConfig) -> Result<LkmStatus> {
    let module_name = loaded_module_name()?;
    let managed = module_name.as_deref() == Some(defs::KASUMI_LKM_MODULE_NAME);
    let current_kmi = current_kmi().unwrap_or_else(|error| format!("unavailable: {error:#}"));
    let module_file = resolve_module_file(config).unwrap_or_default();
    Ok(LkmStatus {
        loaded: module_name.is_some(),
        managed,
        module_name,
        autoload: config.lkm_autoload,
        kmi_override: config.lkm_kmi_override.clone(),
        current_kmi,
        search_dir: config.lkm_dir.clone(),
        module_file,
    })
}
''',
)

types = Path("webui/src/lib/types.ts")
replace_once(
    types,
    '''export interface KasumiLkmStatus {
  loaded: boolean;
  module_name: string | null;
''',
    '''export interface KasumiLkmStatus {
  loaded: boolean;
  managed: boolean;
  module_name: string | null;
''',
)

lkm_ui = Path("webui/src/routes/KasumiTab/LkmSection.tsx")
replace_once(
    lkm_ui,
    '''  const autoloadText = props.lkm.autoload
    ? uiStore.L.kasumi.autoloadOn
    : uiStore.L.kasumi.autoloadOff;

  return (
''',
    '''  const autoloadText = props.lkm.autoload
    ? uiStore.L.kasumi.autoloadOn
    : uiStore.L.kasumi.autoloadOff;
  const externallyManaged = props.lkm.loaded && !props.lkm.managed;

  return (
''',
)
replace_once(
    lkm_ui,
    '''          disabled={props.pending}
          onClick={() =>
            props.lkm.loaded
              ? props.onShowUnloadWarning()
              : props.runAction(
                  () => API.loadKasumiLkm(),
                  uiStore.L.kasumi.loadLkm,
                )
          }
''',
    '''          disabled={props.pending || externallyManaged}
          onClick={() =>
            props.lkm.loaded
              ? props.onShowUnloadWarning()
              : props.runAction(
                  () => API.loadKasumiLkm(),
                  uiStore.L.kasumi.loadLkm,
                )
          }
''',
)
