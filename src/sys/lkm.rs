// Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-only

#[cfg(any(target_os = "linux", target_os = "android"))]
use std::{ffi::CString, os::fd::AsRawFd};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    thread,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::{
    conf::schema::KasumiConfig,
    defs,
    sys::kasumi::{
        self, KSM_FEATURE_CMDLINE_SPOOF, KSM_FEATURE_MAPS_SPOOF, KSM_FEATURE_MOUNT_HIDE,
        KSM_FEATURE_SELINUX_FIX, KSM_FEATURE_STATFS_SPOOF, KSM_FEATURE_UNAME_SPOOF,
        KasumiSpoofUname,
    },
};

#[derive(Debug, Clone, Serialize, Default, PartialEq, Eq)]
pub struct LkmStatus {
    pub loaded: bool,
    pub managed: bool,
    pub module_name: Option<String>,
    pub autoload: bool,
    pub kmi_override: String,
    pub current_kmi: String,
    pub search_dir: PathBuf,
    pub module_file: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ManagedLkmSession {
    module_name: String,
    module_file: PathBuf,
}

static MANAGED_LKM_SESSION: OnceLock<Mutex<Option<ManagedLkmSession>>> = OnceLock::new();

#[cfg(all(
    any(target_os = "linux", target_os = "android"),
    target_arch = "aarch64"
))]
const SYS_FINIT_MODULE_NUM: libc::c_long = 273;
#[cfg(all(any(target_os = "linux", target_os = "android"), target_arch = "arm"))]
const SYS_FINIT_MODULE_NUM: libc::c_long = 379;
#[cfg(all(
    any(target_os = "linux", target_os = "android"),
    any(target_arch = "x86_64", target_arch = "x86")
))]
const SYS_FINIT_MODULE_NUM: libc::c_long = 313;

#[cfg(all(
    any(target_os = "linux", target_os = "android"),
    target_arch = "aarch64"
))]
const SYS_DELETE_MODULE_NUM: libc::c_long = 106;
#[cfg(all(
    any(target_os = "linux", target_os = "android"),
    any(target_arch = "x86_64", target_arch = "x86")
))]
const SYS_DELETE_MODULE_NUM: libc::c_long = 176;
#[cfg(all(any(target_os = "linux", target_os = "android"), target_arch = "arm"))]
const SYS_DELETE_MODULE_NUM: libc::c_long = 129;

fn managed_session() -> &'static Mutex<Option<ManagedLkmSession>> {
    MANAGED_LKM_SESSION.get_or_init(|| Mutex::new(None))
}

fn record_managed_session(session: ManagedLkmSession) -> Result<()> {
    *managed_session()
        .lock()
        .map_err(|_| anyhow::anyhow!("managed Kasumi LKM session lock is poisoned"))? = Some(session);
    Ok(())
}

fn clear_managed_session() -> Result<()> {
    *managed_session()
        .lock()
        .map_err(|_| anyhow::anyhow!("managed Kasumi LKM session lock is poisoned"))? = None;
    Ok(())
}

fn session_manages_loaded_module(
    session: Option<&ManagedLkmSession>,
    loaded_module_name: Option<&str>,
) -> bool {
    matches!(
        (session, loaded_module_name),
        (Some(session), Some(loaded)) if session.module_name == loaded
    )
}

fn read_first_line(path: &Path) -> Result<String> {
    let content =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let line = content
        .lines()
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .with_context(|| format!("{} does not contain a value", path.display()))?;
    Ok(line.to_string())
}

fn arch_suffix() -> &'static str {
    #[cfg(target_arch = "aarch64")]
    {
        "_arm64"
    }
    #[cfg(target_arch = "arm")]
    {
        "_armv7"
    }
    #[cfg(target_arch = "x86_64")]
    {
        "_x86_64"
    }
    #[cfg(target_arch = "x86")]
    {
        "_x86"
    }
}

fn parse_kmi_from_release(release: &str) -> Result<String> {
    let full_version = release.trim();
    if full_version.is_empty() {
        bail!("kernel release is empty");
    }

    let dot1 = full_version
        .find('.')
        .context("kernel release has no major/minor separator")?;
    let dot2 = full_version[dot1 + 1..]
        .find('.')
        .map(|offset| dot1 + 1 + offset)
        .unwrap_or(full_version.len());
    let major_minor = &full_version[..dot2];

    let android_pos = full_version
        .find("-android")
        .context("kernel release has no Android version")?;
    let ver_start = android_pos + "-android".len();
    let ver_end = full_version[ver_start..]
        .find('-')
        .map(|offset| ver_start + offset)
        .unwrap_or(full_version.len());
    let android_ver = &full_version[ver_start..ver_end];

    if android_ver.is_empty() {
        bail!("kernel release has an empty Android version");
    }
    Ok(format!("android{}-{}", android_ver, major_minor))
}

fn real_kernel_release() -> Result<String> {
    read_first_line(Path::new("/proc/sys/kernel/osrelease"))
}

pub fn current_kmi() -> Result<String> {
    parse_kmi_from_release(&real_kernel_release()?)
}

fn effective_kmi(config: &KasumiConfig) -> Result<String> {
    if !config.lkm_kmi_override.trim().is_empty() {
        Ok(config.lkm_kmi_override.trim().to_string())
    } else {
        current_kmi()
    }
}

fn resolve_module_file(config: &KasumiConfig) -> Result<PathBuf> {
    if !config.lkm_dir.is_dir() {
        bail!(
            "Kasumi LKM directory does not exist: {}",
            config.lkm_dir.display()
        );
    }
    let kmi = effective_kmi(config)?;
    let path = config
        .lkm_dir
        .join(format!("{kmi}{}_kasumi_lkm.ko", arch_suffix()));
    if !path.is_file() {
        bail!(
            "canonical Kasumi LKM file does not exist: {}",
            path.display()
        );
    }
    Ok(path)
}

fn loaded_module_name_from_proc_modules(content: &str) -> Option<&str> {
    content.lines().find_map(|line| {
        let name = line.split_whitespace().next()?;
        matches!(name, "kasumi_lkm" | "kasumi").then_some(name)
    })
}

fn loaded_module_name() -> Result<Option<String>> {
    let content = fs::read_to_string("/proc/modules").context("failed to read /proc/modules")?;
    Ok(loaded_module_name_from_proc_modules(&content).map(ToString::to_string))
}

fn managed_module_name() -> Result<Option<String>> {
    let loaded = loaded_module_name()?;
    let session = managed_session()
        .lock()
        .map_err(|_| anyhow::anyhow!("managed Kasumi LKM session lock is poisoned"))?;
    if session_manages_loaded_module(session.as_ref(), loaded.as_deref()) {
        Ok(loaded)
    } else {
        Ok(None)
    }
}

pub fn is_loaded() -> Result<bool> {
    Ok(loaded_module_name()?.is_some())
}

pub fn status(config: &KasumiConfig) -> Result<LkmStatus> {
    let module_name = loaded_module_name()?;
    let session = managed_session()
        .lock()
        .map_err(|_| anyhow::anyhow!("managed Kasumi LKM session lock is poisoned"))?;
    let managed = session_manages_loaded_module(session.as_ref(), module_name.as_deref());
    let current_kmi = current_kmi().unwrap_or_else(|error| format!("unavailable: {error:#}"));
    let module_file = if managed {
        session
            .as_ref()
            .map(|session| session.module_file.clone())
            .unwrap_or_default()
    } else {
        resolve_module_file(config).unwrap_or_default()
    };
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

#[cfg(any(target_os = "linux", target_os = "android"))]
fn load_module_via_finit(ko_path: &Path, params: &str) -> Result<()> {
    let file = fs::File::open(ko_path)
        .with_context(|| format!("failed to open module {}", ko_path.display()))?;
    let params = CString::new(params).context("module params contain interior NUL")?;

    let ret = unsafe { libc::syscall(SYS_FINIT_MODULE_NUM, file.as_raw_fd(), params.as_ptr(), 0) };
    if ret != 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("finit_module failed for {}", ko_path.display()));
    }

    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn load_module_via_finit(_ko_path: &Path, _params: &str) -> Result<()> {
    bail!("kernel module loading is only supported on linux/android")
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn unload_module_via_syscall(module_name: &str) -> Result<()> {
    let module_name = CString::new(module_name).context("module name contains interior NUL")?;
    let ret = unsafe { libc::syscall(SYS_DELETE_MODULE_NUM, module_name.as_ptr(), 0) };
    if ret != 0 {
        return Err(std::io::Error::last_os_error()).context("delete_module failed");
    }
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn unload_module_via_syscall(_module_name: &str) -> Result<()> {
    bail!("kernel module unloading is only supported on linux/android")
}

fn feature_supported(features: i32, feature: i32) -> bool {
    features & feature != 0
}

fn record_cleanup_error(errors: &mut Vec<String>, operation: &str, result: Result<()>) {
    if let Err(error) = result {
        errors.push(format!("{operation}: {error:#}"));
    }
}

fn cleanup_runtime_before_unload() -> Result<()> {
    let features = kasumi::get_features().context("failed to query Kasumi features before unload")?;
    let mut errors = Vec::new();

    record_cleanup_error(&mut errors, "disable runtime", kasumi::set_enabled(false));
    record_cleanup_error(&mut errors, "clear mount rules", kasumi::clear_rules());
    record_cleanup_error(&mut errors, "clear maps rules", kasumi::clear_maps_rules());
    record_cleanup_error(&mut errors, "disable debug", kasumi::set_debug(false));
    record_cleanup_error(&mut errors, "disable stealth", kasumi::set_stealth(false));
    record_cleanup_error(
        &mut errors,
        "clear hidden UID policy",
        kasumi::set_hide_uids(&[]),
    );

    if feature_supported(features, KSM_FEATURE_MOUNT_HIDE) {
        record_cleanup_error(
            &mut errors,
            "disable mount hide",
            kasumi::set_mount_hide(false),
        );
    }
    if feature_supported(features, KSM_FEATURE_MAPS_SPOOF) {
        record_cleanup_error(
            &mut errors,
            "disable maps spoof",
            kasumi::set_maps_spoof(false),
        );
    }
    if feature_supported(features, KSM_FEATURE_STATFS_SPOOF) {
        record_cleanup_error(
            &mut errors,
            "disable statfs spoof",
            kasumi::set_statfs_spoof(false),
        );
    }
    if feature_supported(features, KSM_FEATURE_SELINUX_FIX) {
        record_cleanup_error(
            &mut errors,
            "disable SELinux fix",
            kasumi::set_selinux_fix(false),
        );
    }
    if feature_supported(features, KSM_FEATURE_CMDLINE_SPOOF) {
        record_cleanup_error(
            &mut errors,
            "clear cmdline spoof",
            kasumi::set_cmdline_str(""),
        );
    }
    if feature_supported(features, KSM_FEATURE_UNAME_SPOOF) {
        let empty_uname = KasumiSpoofUname::default();
        record_cleanup_error(
            &mut errors,
            "clear scoped uname spoof",
            kasumi::set_uname(&empty_uname),
        );
        record_cleanup_error(
            &mut errors,
            "restore global uname",
            kasumi::restore_uname_global(),
        );
    }

    if !errors.is_empty() {
        bail!(
            "refusing to unload Kasumi after incomplete runtime cleanup: {}",
            errors.join(" | ")
        );
    }
    Ok(())
}

pub fn load(config: &KasumiConfig) -> Result<()> {
    if kasumi::can_operate()? {
        crate::scoped_log!(
            info,
            "lkm",
            "load skipped: an operable Kasumi runtime is already present"
        );
        return Ok(());
    }
    if is_loaded()? {
        kasumi::invalidate_status_cache()?;
        let status = kasumi::check_status()?;
        bail!(
            "a Kasumi module is already loaded but is not operable (status={}); refusing to report load success",
            kasumi::status_name(status)
        );
    }

    let ko_path = resolve_module_file(config)?;
    let kmi = effective_kmi(config)?;

    load_module_via_finit(&ko_path, "")?;
    kasumi::invalidate_status_cache()?;

    let module_name = loaded_module_name()?.context("Kasumi LKM load returned without a module entry")?;
    if module_name != defs::KASUMI_LKM_MODULE_NAME {
        bail!(
            "loaded unexpected Kasumi module name {module_name}; expected {}; refusing automatic unload because ownership cannot be proven",
            defs::KASUMI_LKM_MODULE_NAME
        );
    }

    if !kasumi::can_operate()? {
        let status = kasumi::check_status()?;
        let validation_error = anyhow::anyhow!(
            "loaded Kasumi LKM failed protocol validation (status={})",
            kasumi::status_name(status)
        );
        let _ = kasumi::release_connection();
        return match unload_module_via_syscall(&module_name) {
            Ok(()) => Err(validation_error.context("invalid LKM was unloaded")),
            Err(unload_error) => bail!(
                "Kasumi LKM validation and cleanup both failed: validation={validation_error:#}; unload={unload_error:#}"
            ),
        };
    }

    let session = ManagedLkmSession {
        module_name: module_name.clone(),
        module_file: ko_path.clone(),
    };
    if let Err(record_error) = record_managed_session(session) {
        let cleanup_error = cleanup_runtime_before_unload().err();
        let _ = kasumi::release_connection();
        let unload_error = unload_module_via_syscall(&module_name).err();
        bail!(
            "Kasumi LKM loaded but ownership registration failed: register={record_error:#}; cleanup={}; unload={}",
            cleanup_error
                .map(|error| format!("{error:#}"))
                .unwrap_or_else(|| "ok".to_string()),
            unload_error
                .map(|error| format!("{error:#}"))
                .unwrap_or_else(|| "ok".to_string())
        );
    }

    crate::scoped_log!(
        info,
        "lkm",
        "load complete: module={}, file={}, kmi={}, ownership=current_daemon_session",
        module_name,
        ko_path.display(),
        kmi
    );
    Ok(())
}

pub fn unload(_config: &KasumiConfig) -> Result<()> {
    let Some(module_name) = managed_module_name()? else {
        if kasumi::can_operate()? || is_loaded()? {
            bail!(
                "active Kasumi runtime was not loaded by this daemon session; refusing to unload it"
            );
        }
        kasumi::release_connection()?;
        clear_managed_session()?;
        return Ok(());
    };

    cleanup_runtime_before_unload()?;
    kasumi::release_connection()?;
    thread::sleep(Duration::from_millis(120));

    let mut last_retry_error = None;
    for _ in 0..5 {
        match unload_module_via_syscall(&module_name) {
            Ok(()) => {
                clear_managed_session()?;
                kasumi::invalidate_status_cache()?;
                crate::scoped_log!(info, "lkm", "unload complete: module={}", module_name);
                return Ok(());
            }
            Err(err) => {
                let retryable = err
                    .downcast_ref::<std::io::Error>()
                    .and_then(|io_err| io_err.raw_os_error())
                    .is_some_and(|code| code == libc::EAGAIN || code == libc::EBUSY);
                last_retry_error = Some(err);
                if !retryable {
                    break;
                }
                thread::sleep(Duration::from_millis(120));
            }
        }
    }

    let err = last_retry_error.expect("delete_module retry loop always executes");
    Err(err)
}

pub fn autoload_if_needed(config: &KasumiConfig) -> Result<bool> {
    if !config.enabled || !config.lkm_autoload {
        return Ok(false);
    }
    if kasumi::can_operate()? {
        return Ok(false);
    }
    if is_loaded()? {
        kasumi::invalidate_status_cache()?;
        let status = kasumi::check_status()?;
        bail!(
            "Kasumi autoload found a loaded but inoperable module (status={})",
            kasumi::status_name(status)
        );
    }
    if kasumi::check_status()? == kasumi::KasumiStatus::KernelNotSupported {
        return Ok(false);
    }

    load(config)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        ManagedLkmSession, loaded_module_name_from_proc_modules, parse_kmi_from_release,
        session_manages_loaded_module,
    };

    #[test]
    fn parses_gki_release() {
        assert_eq!(
            parse_kmi_from_release("5.15.153-android13-8-g123456789abc").unwrap(),
            "android13-5.15"
        );
    }

    #[test]
    fn rejects_vendor_release_without_android_kmi() {
        let error = parse_kmi_from_release("5.15.207-g3ddad1147e36").unwrap_err();

        assert!(error.to_string().contains("has no Android version"));
    }

    #[test]
    fn detects_legacy_and_current_kasumi_modules() {
        assert_eq!(
            loaded_module_name_from_proc_modules("kasumi_lkm 1 0 - Live 0x0\n"),
            Some("kasumi_lkm")
        );
        assert_eq!(
            loaded_module_name_from_proc_modules("kasumi 1 0 - Live 0x0\n"),
            Some("kasumi")
        );
        assert_eq!(
            loaded_module_name_from_proc_modules("other 1 0 - Live 0x0\n"),
            None
        );
    }

    #[test]
    fn module_name_alone_never_grants_unload_ownership() {
        assert!(!session_manages_loaded_module(None, Some("kasumi_lkm")));
    }

    #[test]
    fn ownership_requires_the_current_session_and_matching_module() {
        let session = ManagedLkmSession {
            module_name: "kasumi_lkm".to_string(),
            module_file: PathBuf::from("/tmp/kasumi_lkm.ko"),
        };

        assert!(session_manages_loaded_module(
            Some(&session),
            Some("kasumi_lkm")
        ));
        assert!(!session_manages_loaded_module(
            Some(&session),
            Some("kasumi")
        ));
        assert!(!session_manages_loaded_module(Some(&session), None));
    }
}
