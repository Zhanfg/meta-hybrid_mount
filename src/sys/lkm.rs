// Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-only

#[cfg(any(target_os = "linux", target_os = "android"))]
use std::{ffi::CString, os::fd::AsRawFd};
use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    thread,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::{
    conf::schema::KasumiConfig,
    defs,
    sys::kasumi::{
        self, KSM_FEATURE_CMDLINE_SPOOF, KSM_FEATURE_MAPS_SPOOF, KSM_FEATURE_MOUNT_HIDE,
        KSM_FEATURE_SELINUX_FIX, KSM_FEATURE_STATFS_SPOOF, KSM_FEATURE_UNAME_SPOOF,
        KasumiSpoofUname,
    },
};

const MAX_LKM_BYTES: u64 = 16 * 1024 * 1024;
const MAX_OWNER_RECEIPT_BYTES: u64 = 16 * 1024;
const SUPPORTED_KMIS: &[&str] = &[
    "android12-5.10",
    "android13-5.10",
    "android13-5.15",
    "android14-5.15",
    "android14-6.1",
    "android15-6.6",
    "android16-6.12",
];

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ManagedLkmSession {
    boot_id: String,
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

fn remove_managed_receipt() -> Result<()> {
    match fs::remove_file(defs::KASUMI_LKM_OWNER_FILE) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| {
            format!(
                "failed to remove Kasumi LKM ownership receipt {}",
                defs::KASUMI_LKM_OWNER_FILE
            )
        }),
    }
}

fn cached_managed_session() -> Result<Option<ManagedLkmSession>> {
    Ok(managed_session()
        .lock()
        .map_err(|_| anyhow::anyhow!("managed Kasumi LKM session lock is poisoned"))?
        .clone())
}

fn record_managed_session(session: ManagedLkmSession) -> Result<()> {
    crate::sys::trusted::ensure_private_dir(Path::new(defs::RUN_DIR), 0o700)
        .context("failed to validate private runtime directory before LKM ownership write")?;
    let payload = serde_json::to_vec_pretty(&session)
        .context("failed to serialize Kasumi LKM ownership receipt")?;
    crate::sys::fs::atomic_write(defs::KASUMI_LKM_OWNER_FILE, payload).with_context(|| {
        format!(
            "failed to persist Kasumi LKM ownership receipt {}",
            defs::KASUMI_LKM_OWNER_FILE
        )
    })?;
    crate::sys::trusted::validate_private_regular(
        Path::new(defs::KASUMI_LKM_OWNER_FILE),
        MAX_OWNER_RECEIPT_BYTES,
    )
    .context("persisted Kasumi LKM ownership receipt failed trusted-file validation")?;
    *managed_session()
        .lock()
        .map_err(|_| anyhow::anyhow!("managed Kasumi LKM session lock is poisoned"))? =
        Some(session);
    Ok(())
}

fn preflight_managed_receipt_storage() -> Result<()> {
    crate::sys::trusted::ensure_private_dir(Path::new(defs::RUN_DIR), 0o700)
        .context("failed to validate private runtime directory before LKM load")?;
    let probe = Path::new(defs::RUN_DIR).join(format!(
        ".kasumi_lkm_owner.preflight.{}",
        std::process::id()
    ));
    let result = (|| -> Result<()> {
        crate::sys::fs::atomic_write(&probe, b"rehybird-lkm-owner-preflight")
            .context("failed to create Kasumi ownership preflight file")?;
        crate::sys::trusted::validate_private_regular(&probe, 1024)
            .context("Kasumi ownership preflight file failed trusted-file validation")?;
        fs::remove_file(&probe).context("failed to remove Kasumi ownership preflight file")?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&probe);
    }
    result
}

fn clear_managed_session() -> Result<()> {
    *managed_session()
        .lock()
        .map_err(|_| anyhow::anyhow!("managed Kasumi LKM session lock is poisoned"))? = None;
    remove_managed_receipt()
}

fn session_manages_loaded_module(
    session: Option<&ManagedLkmSession>,
    loaded_module_name: Option<&str>,
    boot_id: &str,
) -> bool {
    matches!(
        (session, loaded_module_name),
        (Some(session), Some(loaded))
            if session.boot_id == boot_id
                && session.module_name == defs::KASUMI_LKM_MODULE_NAME
                && session.module_name == loaded
                && session.module_file.is_absolute()
                && session.module_file.starts_with(defs::KASUMI_LKM_DIR)
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

fn current_boot_id() -> Result<String> {
    read_first_line(Path::new("/proc/sys/kernel/random/boot_id"))
}

fn load_managed_session() -> Result<Option<ManagedLkmSession>> {
    let boot_id = current_boot_id().context("failed to read current boot id")?;
    let cached_session = cached_managed_session()?;

    if let Some(session) = cached_session {
        if session.boot_id == boot_id {
            return Ok(Some(session));
        }
        clear_managed_session()?;
    }

    match fs::symlink_metadata(defs::KASUMI_LKM_OWNER_FILE) {
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "failed to inspect Kasumi LKM ownership receipt {}",
                    defs::KASUMI_LKM_OWNER_FILE
                )
            });
        }
    }

    let content = match crate::sys::trusted::read_private_text(
        Path::new(defs::KASUMI_LKM_OWNER_FILE),
        MAX_OWNER_RECEIPT_BYTES,
    ) {
        Ok(content) => content,
        Err(error) => {
            crate::scoped_log!(
                warn,
                "lkm",
                "ownership receipt is not a trusted private file; treating module as unmanaged and removing only the receipt path: path={}, error={:#}",
                defs::KASUMI_LKM_OWNER_FILE,
                error
            );
            remove_managed_receipt()?;
            return Ok(None);
        }
    };

    let session: ManagedLkmSession = match serde_json::from_str(&content) {
        Ok(session) => session,
        Err(error) => {
            crate::scoped_log!(
                warn,
                "lkm",
                "ownership receipt invalid; removing it: path={}, error={}",
                defs::KASUMI_LKM_OWNER_FILE,
                error
            );
            remove_managed_receipt()?;
            return Ok(None);
        }
    };

    if session.boot_id != boot_id
        || session.module_name != defs::KASUMI_LKM_MODULE_NAME
        || !session.module_file.is_absolute()
        || !session.module_file.starts_with(defs::KASUMI_LKM_DIR)
    {
        crate::scoped_log!(
            debug,
            "lkm",
            "discard stale ownership receipt: path={}, receipt_boot={}, current_boot={}, module={}",
            defs::KASUMI_LKM_OWNER_FILE,
            session.boot_id,
            boot_id,
            session.module_name
        );
        remove_managed_receipt()?;
        return Ok(None);
    }

    *managed_session()
        .lock()
        .map_err(|_| anyhow::anyhow!("managed Kasumi LKM session lock is poisoned"))? =
        Some(session.clone());
    Ok(Some(session))
}

fn managed_session_for_loaded_module(
    loaded_module_name: Option<&str>,
) -> Result<Option<ManagedLkmSession>> {
    let boot_id = current_boot_id().context("failed to read current boot id")?;
    let session = load_managed_session()?;
    if session_manages_loaded_module(session.as_ref(), loaded_module_name, &boot_id) {
        return Ok(session);
    }

    if session.is_some() {
        crate::scoped_log!(
            warn,
            "lkm",
            "ownership receipt no longer matches the loaded module; revoking management"
        );
        clear_managed_session()?;
    }
    Ok(None)
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
    let mut version_parts = major_minor.split('.');
    let major = version_parts.next().context("kernel release has no major version")?;
    let minor = version_parts.next().context("kernel release has no minor version")?;
    if version_parts.next().is_some()
        || major.is_empty()
        || minor.is_empty()
        || !major.bytes().all(|byte| byte.is_ascii_digit())
        || !minor.bytes().all(|byte| byte.is_ascii_digit())
    {
        bail!("kernel release has an invalid major.minor version");
    }

    let android_pos = full_version
        .find("-android")
        .context("kernel release has no Android version")?;
    let ver_start = android_pos + "-android".len();
    let ver_end = full_version[ver_start..]
        .find('-')
        .map(|offset| ver_start + offset)
        .unwrap_or(full_version.len());
    let android_ver = &full_version[ver_start..ver_end];

    if android_ver.is_empty() || !android_ver.bytes().all(|byte| byte.is_ascii_digit()) {
        bail!("kernel release has an invalid Android version");
    }
    Ok(format!("android{android_ver}-{major}.{minor}"))
}

fn real_kernel_release() -> Result<String> {
    read_first_line(Path::new("/proc/sys/kernel/osrelease"))
}

pub fn current_kmi() -> Result<String> {
    parse_kmi_from_release(&real_kernel_release()?)
}

fn effective_kmi(config: &KasumiConfig) -> Result<String> {
    let kmi = if !config.lkm_kmi_override.trim().is_empty() {
        crate::sys::kmi_guard::validate_override(config)?;
        config.lkm_kmi_override.trim().to_string()
    } else {
        current_kmi()?
    };

    if !SUPPORTED_KMIS.contains(&kmi.as_str()) {
        bail!("Kasumi KMI is not in the audited package target set: {kmi}");
    }
    Ok(kmi)
}

fn resolve_module_file(config: &KasumiConfig) -> Result<PathBuf> {
    if config.lkm_dir != Path::new(defs::KASUMI_LKM_DIR) {
        bail!(
            "Kasumi LKM autoload directory must be the audited package directory: configured={}, required={}",
            config.lkm_dir.display(),
            defs::KASUMI_LKM_DIR
        );
    }

    let dir_metadata = fs::symlink_metadata(&config.lkm_dir).with_context(|| {
        format!(
            "failed to inspect Kasumi LKM directory {}",
            config.lkm_dir.display()
        )
    })?;
    if !dir_metadata.file_type().is_dir() {
        bail!(
            "Kasumi LKM directory is not a real directory: {}",
            config.lkm_dir.display()
        );
    }
    let canonical_dir = fs::canonicalize(&config.lkm_dir).with_context(|| {
        format!(
            "failed to canonicalize Kasumi LKM directory {}",
            config.lkm_dir.display()
        )
    })?;
    if canonical_dir != config.lkm_dir {
        bail!(
            "Kasumi LKM directory resolves through a different path: {} -> {}",
            config.lkm_dir.display(),
            canonical_dir.display()
        );
    }

    let kmi = effective_kmi(config)?;
    let path = config
        .lkm_dir
        .join(format!("{kmi}{}_kasumi_lkm.ko", arch_suffix()));
    crate::sys::trusted::validate_private_regular(&path, MAX_LKM_BYTES)
        .with_context(|| format!("Kasumi LKM file is not trusted: {}", path.display()))?;
    let canonical_file = fs::canonicalize(&path)
        .with_context(|| format!("failed to canonicalize Kasumi LKM file {}", path.display()))?;
    if canonical_file != path {
        bail!(
            "Kasumi LKM file resolves through a different path: {} -> {}",
            path.display(),
            canonical_file.display()
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
    if managed_session_for_loaded_module(loaded.as_deref())?.is_some() {
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
    let session = managed_session_for_loaded_module(module_name.as_deref())?;
    let managed = session.is_some();
    let current_kmi = current_kmi().unwrap_or_else(|error| format!("unavailable: {error:#}"));
    let module_file = if let Some(session) = session {
        session.module_file
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
    let features =
        kasumi::get_features().context("failed to query Kasumi features before unload")?;
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

fn rollback_newly_loaded_module(primary_error: anyhow::Error) -> Result<()> {
    let cleanup_error = cleanup_runtime_before_unload().err();
    let release_error = kasumi::release_connection().err();
    let unload_error = unload_module_via_syscall(defs::KASUMI_LKM_MODULE_NAME).err();
    let receipt_error = if unload_error.is_none() {
        clear_managed_session().err()
    } else {
        None
    };

    bail!(
        "Kasumi LKM post-load transaction failed: primary={primary_error:#}; runtime_cleanup={}; release_connection={}; unload={}; receipt_cleanup={}; ownership_receipt_retained_if_unload_failed=true",
        cleanup_error
            .map(|error| format!("{error:#}"))
            .unwrap_or_else(|| "ok".to_string()),
        release_error
            .map(|error| format!("{error:#}"))
            .unwrap_or_else(|| "ok".to_string()),
        unload_error
            .map(|error| format!("{error:#}"))
            .unwrap_or_else(|| "ok".to_string()),
        receipt_error
            .map(|error| format!("{error:#}"))
            .unwrap_or_else(|| "ok".to_string())
    )
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

    // Everything that can be checked without changing kernel state must happen
    // before finit_module(). This leaves the smallest possible post-load failure
    // surface and ensures every successful load can be bound to this boot.
    let boot_id = current_boot_id().context("failed to bind prospective LKM ownership to boot")?;
    let ko_path = resolve_module_file(config)?;
    let kmi = effective_kmi(config)?;
    preflight_managed_receipt_storage()?;
    let session = ManagedLkmSession {
        boot_id,
        module_name: defs::KASUMI_LKM_MODULE_NAME.to_string(),
        module_file: ko_path.clone(),
    };

    load_module_via_finit(&ko_path, "")?;

    // Persist ownership immediately after the kernel accepts the module. Any
    // later failure can then safely retain the receipt when delete_module fails.
    if let Err(record_error) = record_managed_session(session) {
        return rollback_newly_loaded_module(
            record_error.context("loaded Kasumi LKM but ownership registration failed"),
        );
    }

    if let Err(error) = kasumi::invalidate_status_cache() {
        return rollback_newly_loaded_module(
            error.context("failed to invalidate Kasumi status cache after LKM load"),
        );
    }

    let module_name = match loaded_module_name() {
        Ok(Some(module_name)) => module_name,
        Ok(None) => {
            return rollback_newly_loaded_module(anyhow::anyhow!(
                "Kasumi LKM load returned without a module entry"
            ));
        }
        Err(error) => {
            return rollback_newly_loaded_module(
                error.context("failed to verify /proc/modules after Kasumi LKM load"),
            );
        }
    };
    if module_name != defs::KASUMI_LKM_MODULE_NAME {
        return rollback_newly_loaded_module(anyhow::anyhow!(
            "loaded unexpected Kasumi module name {module_name}; expected {}",
            defs::KASUMI_LKM_MODULE_NAME
        ));
    }

    match kasumi::can_operate() {
        Ok(true) => {}
        Ok(false) => {
            let status = kasumi::check_status()
                .map(|status| kasumi::status_name(status).to_string())
                .unwrap_or_else(|error| format!("status-query-error:{error:#}"));
            return rollback_newly_loaded_module(anyhow::anyhow!(
                "loaded Kasumi LKM failed protocol validation (status={status})"
            ));
        }
        Err(error) => {
            return rollback_newly_loaded_module(
                error.context("Kasumi protocol operability check failed after LKM load"),
            );
        }
    }

    crate::scoped_log!(
        info,
        "lkm",
        "load complete: module={}, file={}, kmi={}, ownership=current_boot_receipt",
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
                "active Kasumi runtime has no valid current-boot ownership receipt; refusing to unload it"
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
                let clear_result = clear_managed_session();
                kasumi::invalidate_status_cache()?;
                clear_result?;
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
        ManagedLkmSession, SUPPORTED_KMIS, loaded_module_name_from_proc_modules,
        parse_kmi_from_release, session_manages_loaded_module,
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
    fn rejects_non_numeric_gki_markers() {
        assert!(parse_kmi_from_release("6.x.1-android15-8-g123").is_err());
        assert!(parse_kmi_from_release("6.6.1-androidx-8-g123").is_err());
    }

    #[test]
    fn audited_kmi_set_is_exact() {
        assert_eq!(
            SUPPORTED_KMIS,
            [
                "android12-5.10",
                "android13-5.10",
                "android13-5.15",
                "android14-5.15",
                "android14-6.1",
                "android15-6.6",
                "android16-6.12",
            ]
        );
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
        assert!(!session_manages_loaded_module(
            None,
            Some("kasumi_lkm"),
            "boot-a"
        ));
    }

    #[test]
    fn ownership_requires_current_boot_canonical_module_and_packaged_file() {
        let session = ManagedLkmSession {
            boot_id: "boot-a".to_string(),
            module_name: "kasumi_lkm".to_string(),
            module_file: PathBuf::from(
                "/data/adb/modules/hybrid_mount/kasumi_lkm/android15-6.6_arm64_kasumi_lkm.ko",
            ),
        };

        assert!(session_manages_loaded_module(
            Some(&session),
            Some("kasumi_lkm"),
            "boot-a"
        ));
        assert!(!session_manages_loaded_module(
            Some(&session),
            Some("kasumi"),
            "boot-a"
        ));
        assert!(!session_manages_loaded_module(
            Some(&session),
            Some("kasumi_lkm"),
            "boot-b"
        ));
        assert!(!session_manages_loaded_module(
            Some(&session),
            None,
            "boot-a"
        ));

        let outside_package = ManagedLkmSession {
            module_file: PathBuf::from("/tmp/kasumi_lkm.ko"),
            ..session
        };
        assert!(!session_manages_loaded_module(
            Some(&outside_package),
            Some("kasumi_lkm"),
            "boot-a"
        ));
    }
}
