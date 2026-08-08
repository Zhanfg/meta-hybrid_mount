// Copyright (C) 2026 Hybrid Mount Developers
//
// SPDX-License-Identifier: GPL-3.0-only

use std::{
    collections::{BTreeSet, HashSet},
    ffi::CString,
    fs::{self, File, OpenOptions},
    os::{
        fd::AsRawFd,
        unix::{ffi::OsStrExt, fs::FileTypeExt},
    },
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail};

use crate::{core::inventory::Module, defs, partitions};

const DEVICE_PATH: &str = "/dev/zeromount";
const ZEROMOUNT_MAGIC: u32 = 0x5A;
const IOC_NRBITS: u32 = 8;
const IOC_TYPEBITS: u32 = 8;
const IOC_SIZEBITS: u32 = 14;
const IOC_NRSHIFT: u32 = 0;
const IOC_TYPESHIFT: u32 = IOC_NRSHIFT + IOC_NRBITS;
const IOC_SIZESHIFT: u32 = IOC_TYPESHIFT + IOC_TYPEBITS;
const IOC_DIRSHIFT: u32 = IOC_SIZESHIFT + IOC_SIZEBITS;
const IOC_NONE: u32 = 0;
const IOC_WRITE: u32 = 1;
const IOC_READ: u32 = 2;
const ZM_ACTIVE: u32 = 1;
const ZM_DIR: u32 = 128;
const MAX_RULE_LIST_BYTES: usize = 64 * 1024;

#[repr(C)]
struct IoctlData {
    virtual_path: *const libc::c_char,
    real_path: *const libc::c_char,
    flags: u32,
    #[cfg(target_pointer_width = "64")]
    _pad: u32,
}

const IOCTL_DATA_SIZE: u32 = std::mem::size_of::<IoctlData>() as u32;
const IOCTL_ADD_RULE: u32 = iow(ZEROMOUNT_MAGIC, 1, IOCTL_DATA_SIZE);
const IOCTL_DEL_RULE: u32 = iow(ZEROMOUNT_MAGIC, 2, IOCTL_DATA_SIZE);
const IOCTL_GET_VERSION: u32 = ior(ZEROMOUNT_MAGIC, 4, 4);
const IOCTL_GET_LIST: u32 = ior(ZEROMOUNT_MAGIC, 7, 4);
const IOCTL_ENABLE: u32 = io(ZEROMOUNT_MAGIC, 8);
const IOCTL_DISABLE: u32 = io(ZEROMOUNT_MAGIC, 9);
const IOCTL_REFRESH: u32 = io(ZEROMOUNT_MAGIC, 10);
const IOCTL_GET_STATUS: u32 = ior(ZEROMOUNT_MAGIC, 11, 4);

const fn ioc(dir: u32, typ: u32, nr: u32, size: u32) -> u32 {
    (dir << IOC_DIRSHIFT) | (typ << IOC_TYPESHIFT) | (nr << IOC_NRSHIFT) | (size << IOC_SIZESHIFT)
}

const fn io(typ: u32, nr: u32) -> u32 {
    ioc(IOC_NONE, typ, nr, 0)
}

const fn iow(typ: u32, nr: u32, size: u32) -> u32 {
    ioc(IOC_WRITE, typ, nr, size)
}

const fn ior(typ: u32, nr: u32, size: u32) -> u32 {
    ioc(IOC_READ, typ, nr, size)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeState {
    Ready { version: u32 },
    Unavailable { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedirectRule {
    pub virtual_path: PathBuf,
    pub real_path: PathBuf,
    pub is_dir: bool,
}

pub enum ApplyOutcome {
    Applied {
        guard: RuntimeGuard,
        partitions: Vec<String>,
        rule_count: usize,
        version: u32,
    },
    Fallback {
        reason: String,
    },
}

trait DriverOps {
    fn version(&self) -> Result<u32>;
    fn status(&self) -> Result<Option<bool>>;
    fn list_rules(&self) -> Result<String>;
    fn add_rule(&self, rule: &RedirectRule) -> Result<()>;
    fn del_rule(&self, rule: &RedirectRule) -> Result<()>;
    fn enable(&self) -> Result<()>;
    fn disable(&self) -> Result<()>;
    fn refresh(&self) -> Result<()>;
}

struct Driver {
    file: File,
}

impl Driver {
    fn open() -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(DEVICE_PATH)
            .with_context(|| format!("failed to open {DEVICE_PATH}"))?;
        Ok(Self { file })
    }

    fn raw_ioctl(&self, request: u32, arg: *mut libc::c_void) -> Result<i32> {
        // SAFETY: `self.file` owns a valid fd for the ZeroMount device. Each
        // caller below passes the buffer shape required by the published ABI.
        let ret = unsafe { libc::ioctl(self.file.as_raw_fd(), request as libc::c_ulong, arg) };
        if ret < 0 {
            Err(std::io::Error::last_os_error())
                .with_context(|| format!("ZeroMount ioctl 0x{request:08x} failed"))
        } else {
            Ok(ret)
        }
    }

    fn rule_ioctl(&self, request: u32, rule: &RedirectRule, flags: u32) -> Result<()> {
        let virtual_path =
            CString::new(rule.virtual_path.as_os_str().as_bytes()).with_context(|| {
                format!(
                    "invalid ZeroMount virtual path {}",
                    rule.virtual_path.display()
                )
            })?;
        let real_path = CString::new(rule.real_path.as_os_str().as_bytes())
            .with_context(|| format!("invalid ZeroMount real path {}", rule.real_path.display()))?;
        let mut data = IoctlData {
            virtual_path: virtual_path.as_ptr(),
            real_path: real_path.as_ptr(),
            flags,
            #[cfg(target_pointer_width = "64")]
            _pad: 0,
        };
        self.raw_ioctl(
            request,
            (&mut data as *mut IoctlData).cast::<libc::c_void>(),
        )?;
        Ok(())
    }
}

impl DriverOps for Driver {
    fn version(&self) -> Result<u32> {
        let mut version = 0i32;
        let ret = self.raw_ioctl(
            IOCTL_GET_VERSION,
            (&mut version as *mut i32).cast::<libc::c_void>(),
        )?;
        let value = if ret > 0 {
            ret as u32
        } else {
            version.max(0) as u32
        };
        if value == 0 {
            bail!("ZeroMount driver returned version 0");
        }
        Ok(value)
    }

    fn status(&self) -> Result<Option<bool>> {
        let mut status = 0i32;
        match self.raw_ioctl(
            IOCTL_GET_STATUS,
            (&mut status as *mut i32).cast::<libc::c_void>(),
        ) {
            Ok(ret) => Ok(Some(if ret > 0 { ret != 0 } else { status != 0 })),
            Err(error) => {
                let unsupported = error
                    .downcast_ref::<std::io::Error>()
                    .and_then(std::io::Error::raw_os_error)
                    .is_some_and(|errno| errno == libc::ENOTTY || errno == libc::EINVAL);
                if unsupported { Ok(None) } else { Err(error) }
            }
        }
    }

    fn list_rules(&self) -> Result<String> {
        let mut buffer = vec![0u8; MAX_RULE_LIST_BYTES];
        let ret = self.raw_ioctl(IOCTL_GET_LIST, buffer.as_mut_ptr().cast::<libc::c_void>())?;
        let len = usize::try_from(ret).context("negative ZeroMount list length")?;
        if len > buffer.len() {
            bail!("ZeroMount rule list exceeds userspace buffer: {len}");
        }
        buffer.truncate(len);
        String::from_utf8(buffer).context("ZeroMount returned a non UTF-8 rule list")
    }

    fn add_rule(&self, rule: &RedirectRule) -> Result<()> {
        self.rule_ioctl(
            IOCTL_ADD_RULE,
            rule,
            ZM_ACTIVE | if rule.is_dir { ZM_DIR } else { 0 },
        )
    }

    fn del_rule(&self, rule: &RedirectRule) -> Result<()> {
        // Match the published ZeroMount userspace DEL_RULE ABI. Deletion is
        // keyed by virtual path; do not claim directory semantics here.
        self.rule_ioctl(IOCTL_DEL_RULE, rule, ZM_ACTIVE)
    }

    fn enable(&self) -> Result<()> {
        self.raw_ioctl(IOCTL_ENABLE, std::ptr::null_mut())?;
        Ok(())
    }

    fn disable(&self) -> Result<()> {
        self.raw_ioctl(IOCTL_DISABLE, std::ptr::null_mut())?;
        Ok(())
    }

    fn refresh(&self) -> Result<()> {
        self.raw_ioctl(IOCTL_REFRESH, std::ptr::null_mut())?;
        Ok(())
    }
}

pub fn probe_clean_driver() -> Result<ProbeState> {
    if !Path::new(DEVICE_PATH).exists() {
        return Ok(ProbeState::Unavailable {
            reason: "device_missing".to_string(),
        });
    }
    let driver = Driver::open()?;
    probe_driver(&driver)
}

fn probe_driver<D: DriverOps>(driver: &D) -> Result<ProbeState> {
    let version = driver.version()?;
    let Some(enabled) = driver.status()? else {
        return Ok(ProbeState::Unavailable {
            reason: "status_ioctl_missing".to_string(),
        });
    };
    let existing_rules = driver.list_rules()?;
    if enabled || existing_rules.lines().any(|line| !line.trim().is_empty()) {
        return Ok(ProbeState::Unavailable {
            reason: "foreign_runtime_state".to_string(),
        });
    }
    Ok(ProbeState::Ready { version })
}

pub fn module_preflight_supported(module: &Module, managed_partitions: &[String]) -> Result<bool> {
    for partition in managed_partitions {
        let root = module.source_path.join(partition);
        if !root.is_dir() {
            continue;
        }
        if tree_has_unsupported_overlay_semantics(&root)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn tree_has_unsupported_overlay_semantics(root: &Path) -> Result<bool> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("failed to inspect {}", path.display()))?;
        let file_type = metadata.file_type();
        if file_type.is_char_device() {
            return Ok(true);
        }
        if has_overlay_control_xattr(&path)? {
            return Ok(true);
        }
        if file_type.is_dir() {
            for entry in
                fs::read_dir(&path).with_context(|| format!("failed to read {}", path.display()))?
            {
                let entry = entry?;
                let child = entry.path();
                let name = entry.file_name();
                if name == defs::REPLACE_DIR_FILE_NAME || name.as_bytes().starts_with(b".wh.") {
                    return Ok(true);
                }
                stack.push(child);
            }
        }
    }
    Ok(false)
}

fn has_overlay_control_xattr(path: &Path) -> Result<bool> {
    const ATTRS: &[&[u8]] = &[
        b"trusted.overlay.opaque\0",
        b"trusted.overlay.whiteout\0",
        b"trusted.overlay.redirect\0",
    ];
    let c_path = CString::new(path.as_os_str().as_bytes())?;
    for attr in ATTRS {
        // SAFETY: path is a live CString and attr literals are NUL-terminated.
        let len = unsafe {
            libc::lgetxattr(
                c_path.as_ptr(),
                attr.as_ptr().cast::<libc::c_char>(),
                std::ptr::null_mut(),
                0,
            )
        };
        if len >= 0 {
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn apply_modules(modules: &[Module], ids: &[String]) -> Result<ApplyOutcome> {
    let id_set: HashSet<&str> = ids.iter().map(String::as_str).collect();
    let selected = modules
        .iter()
        .filter(|module| id_set.contains(module.id.as_str()))
        .collect::<Vec<_>>();
    if selected.len() != ids.len() {
        return Ok(ApplyOutcome::Fallback {
            reason: "selected_module_missing".to_string(),
        });
    }

    let managed_partitions = partitions::managed_partition_names();
    let (rules, active_partitions) = build_rules(&selected, &managed_partitions)?;
    if rules.is_empty() {
        return Ok(ApplyOutcome::Fallback {
            reason: "no_redirect_rules".to_string(),
        });
    }

    let driver = Driver::open()?;
    let ProbeState::Ready { version } = probe_driver(&driver)? else {
        return Ok(ApplyOutcome::Fallback {
            reason: "driver_not_clean".to_string(),
        });
    };

    match apply_rules_transaction(&driver, &rules) {
        Ok(()) => Ok(ApplyOutcome::Applied {
            guard: RuntimeGuard {
                driver,
                rules: rules.clone(),
                armed: true,
            },
            partitions: active_partitions,
            rule_count: rules.len(),
            version,
        }),
        Err(ApplyFailure::Fallback(reason)) => Ok(ApplyOutcome::Fallback { reason }),
        Err(ApplyFailure::Fatal(error)) => Err(error),
    }
}

fn build_rules(
    modules: &[&Module],
    managed_partitions: &[String],
) -> Result<(Vec<RedirectRule>, Vec<String>)> {
    let mut ordered = modules.to_vec();
    ordered.sort_by(|a, b| b.id.cmp(&a.id));
    let mut rules = Vec::new();
    let mut seen_virtual = HashSet::new();
    let mut active_partitions = BTreeSet::new();

    for module in ordered {
        for partition in managed_partitions {
            let source_root = module.source_path.join(partition);
            if !source_root.is_dir() {
                continue;
            }
            active_partitions.insert(partition.clone());
            collect_rules(
                &module.source_path,
                &source_root,
                &mut rules,
                &mut seen_virtual,
                managed_partitions,
            )?;
        }
    }

    Ok((rules, active_partitions.into_iter().collect()))
}

fn collect_rules(
    module_root: &Path,
    source: &Path,
    rules: &mut Vec<RedirectRule>,
    seen_virtual: &mut HashSet<PathBuf>,
    managed_partitions: &[String],
) -> Result<()> {
    let relative = source
        .strip_prefix(module_root)
        .with_context(|| format!("{} escaped module root", source.display()))?;
    let target = resolve_target_path(relative).context("failed to resolve ZeroMount target")?;
    let metadata = fs::symlink_metadata(source)
        .with_context(|| format!("failed to inspect {}", source.display()))?;
    let file_type = metadata.file_type();

    if is_partition_root(&target, managed_partitions) && !file_type.is_dir() {
        bail!(
            "refusing ZeroMount rule that replaces partition root {}",
            target.display()
        );
    }

    if file_type.is_dir() {
        if !target.exists() {
            if is_partition_root(&target, managed_partitions) {
                bail!("managed partition target is missing: {}", target.display());
            }
            if seen_virtual.insert(target.clone()) {
                rules.push(RedirectRule {
                    virtual_path: target,
                    real_path: source.to_path_buf(),
                    is_dir: true,
                });
            }
            return Ok(());
        }

        let mut entries = fs::read_dir(source)
            .with_context(|| format!("failed to read {}", source.display()))?
            .collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let child = entry.path();
            let name = entry.file_name();
            if name == defs::REPLACE_DIR_FILE_NAME || name.as_bytes().starts_with(b".wh.") {
                bail!(
                    "unsupported ZeroMount overlay marker at {}",
                    child.display()
                );
            }
            collect_rules(module_root, &child, rules, seen_virtual, managed_partitions)?;
        }
        return Ok(());
    }

    if file_type.is_char_device() || has_overlay_control_xattr(source)? {
        bail!(
            "unsupported ZeroMount overlay entry at {}",
            source.display()
        );
    }

    if seen_virtual.insert(target.clone()) {
        rules.push(RedirectRule {
            virtual_path: target,
            real_path: source.to_path_buf(),
            is_dir: false,
        });
    }
    Ok(())
}

fn is_partition_root(target: &Path, managed_partitions: &[String]) -> bool {
    managed_partitions
        .iter()
        .any(|partition| target == Path::new("/").join(partition))
}

pub(crate) fn resolve_target_path(relative: &Path) -> Option<PathBuf> {
    let value = relative.to_str()?;
    if value.is_empty() {
        return None;
    }
    const ALIASES: &[(&str, &str)] = &[
        ("system/vendor", "vendor"),
        ("system/product", "product"),
        ("system/system_ext", "system_ext"),
        ("system/odm", "odm"),
    ];
    for (alias, canonical) in ALIASES {
        if value == *alias {
            return Some(Path::new("/").join(canonical));
        }
        if let Some(rest) = value
            .strip_prefix(alias)
            .and_then(|rest| rest.strip_prefix('/'))
        {
            return Some(Path::new("/").join(canonical).join(rest));
        }
    }
    Some(Path::new("/").join(relative))
}

enum ApplyFailure {
    Fallback(String),
    Fatal(anyhow::Error),
}

fn apply_rules_transaction<D: DriverOps>(
    driver: &D,
    rules: &[RedirectRule],
) -> std::result::Result<(), ApplyFailure> {
    let mut added = Vec::new();
    for rule in rules {
        if let Err(error) = driver.add_rule(rule) {
            return cleanup_after_failure(
                driver,
                &added,
                format!("rule_injection_failed: {error:#}"),
            );
        }
        added.push(rule.clone());
    }
    if let Err(error) = driver.enable() {
        return cleanup_after_failure(driver, &added, format!("enable_failed: {error:#}"));
    }
    if let Err(error) = driver.refresh() {
        return cleanup_after_failure(driver, &added, format!("refresh_failed: {error:#}"));
    }
    Ok(())
}

fn cleanup_after_failure<D: DriverOps>(
    driver: &D,
    owned_rules: &[RedirectRule],
    reason: String,
) -> std::result::Result<(), ApplyFailure> {
    match cleanup_owned_rules(driver, owned_rules) {
        Ok(()) => Err(ApplyFailure::Fallback(reason)),
        Err(error) => Err(ApplyFailure::Fatal(anyhow!(
            "ZeroMount transaction failed and rollback was incomplete: {reason}; rollback={error:#}"
        ))),
    }
}

fn cleanup_owned_rules<D: DriverOps>(driver: &D, owned_rules: &[RedirectRule]) -> Result<()> {
    let mut errors = Vec::new();

    for rule in owned_rules.iter().rev() {
        if let Err(error) = driver.del_rule(rule) {
            errors.push(format!(
                "del_rule({})={error:#}",
                rule.virtual_path.display()
            ));
        }
    }

    let remaining_rules = match driver.list_rules() {
        Ok(value) => Some(value),
        Err(error) => {
            errors.push(format!("list_rules={error:#}"));
            None
        }
    };

    if remaining_rules
        .as_deref()
        .is_some_and(|value| !value.lines().any(|line| !line.trim().is_empty()))
    {
        match driver.status() {
            Ok(Some(true)) => {
                if let Err(error) = driver.disable() {
                    errors.push(format!("disable={error:#}"));
                }
            }
            Ok(Some(false)) => {}
            Ok(None) => errors.push("status_ioctl_missing_during_cleanup".to_string()),
            Err(error) => errors.push(format!("status={error:#}")),
        }
    }

    if let Err(error) = driver.refresh() {
        errors.push(format!("refresh={error:#}"));
    }

    if errors.is_empty() {
        Ok(())
    } else {
        bail!("{}", errors.join(" | "))
    }
}

pub struct RuntimeGuard {
    driver: Driver,
    rules: Vec<RedirectRule>,
    armed: bool,
}

impl RuntimeGuard {
    pub(crate) fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for RuntimeGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        match cleanup_owned_rules(&self.driver, &self.rules) {
            Ok(()) => crate::scoped_log!(
                warn,
                "zeromount",
                "ZeroMount runtime rolled back after later transaction failure"
            ),
            Err(error) => crate::scoped_log!(
                error,
                "zeromount",
                "ZeroMount rollback incomplete: error={:#}",
                error
            ),
        }
    }
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(IOCTL_ADD_RULE == 0x40185A01);
    assert!(IOCTL_DEL_RULE == 0x40185A02);
    assert!(IOCTL_GET_VERSION == 0x80045A04);
    assert!(IOCTL_GET_LIST == 0x80045A07);
    assert!(IOCTL_ENABLE == 0x5A08);
    assert!(IOCTL_DISABLE == 0x5A09);
    assert!(IOCTL_REFRESH == 0x5A0A);
    assert!(IOCTL_GET_STATUS == 0x80045A0B);
};

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::HashMap};

    use tempfile::TempDir;

    use super::*;
    use crate::domain::{ModuleRules, MountMode};

    #[derive(Default)]
    struct MockDriver {
        version: u32,
        enabled: bool,
        rules: String,
        fail_add_at: Option<usize>,
        add_count: RefCell<usize>,
        del_calls: RefCell<usize>,
        enable_calls: RefCell<usize>,
        disable_calls: RefCell<usize>,
        refresh_calls: RefCell<usize>,
    }

    impl DriverOps for MockDriver {
        fn version(&self) -> Result<u32> {
            Ok(self.version)
        }
        fn status(&self) -> Result<Option<bool>> {
            Ok(Some(self.enabled))
        }
        fn list_rules(&self) -> Result<String> {
            Ok(self.rules.clone())
        }
        fn add_rule(&self, _rule: &RedirectRule) -> Result<()> {
            let mut count = self.add_count.borrow_mut();
            *count += 1;
            if self.fail_add_at == Some(*count) {
                bail!("injected failure");
            }
            Ok(())
        }
        fn del_rule(&self, _rule: &RedirectRule) -> Result<()> {
            *self.del_calls.borrow_mut() += 1;
            Ok(())
        }
        fn enable(&self) -> Result<()> {
            *self.enable_calls.borrow_mut() += 1;
            Ok(())
        }
        fn disable(&self) -> Result<()> {
            *self.disable_calls.borrow_mut() += 1;
            Ok(())
        }
        fn refresh(&self) -> Result<()> {
            *self.refresh_calls.borrow_mut() += 1;
            Ok(())
        }
    }

    fn rule(name: &str) -> RedirectRule {
        RedirectRule {
            virtual_path: PathBuf::from(format!("/system/{name}")),
            real_path: PathBuf::from(format!("/data/adb/modules/test/system/{name}")),
            is_dir: false,
        }
    }

    #[test]
    fn foreign_runtime_state_is_never_taken_over() {
        let driver = MockDriver {
            version: 1,
            enabled: true,
            rules: "/system/a -> /data/a".to_string(),
            ..MockDriver::default()
        };
        assert!(matches!(
            probe_driver(&driver).unwrap(),
            ProbeState::Unavailable { ref reason } if reason == "foreign_runtime_state"
        ));
    }

    #[test]
    fn partial_injection_removes_only_owned_rules_and_never_enables() {
        let driver = MockDriver {
            version: 1,
            fail_add_at: Some(2),
            ..MockDriver::default()
        };
        let result = apply_rules_transaction(&driver, &[rule("a"), rule("b")]);
        assert!(matches!(result, Err(ApplyFailure::Fallback(_))));
        assert_eq!(*driver.enable_calls.borrow(), 0);
        assert_eq!(*driver.del_calls.borrow(), 1);
        assert_eq!(*driver.disable_calls.borrow(), 0);
        assert_eq!(*driver.refresh_calls.borrow(), 1);
    }

    #[test]
    fn owned_cleanup_preserves_foreign_rules_and_global_enable() {
        let driver = MockDriver {
            version: 1,
            enabled: true,
            rules: "/system/foreign -> /data/foreign".to_string(),
            ..MockDriver::default()
        };
        cleanup_owned_rules(&driver, &[rule("ours")]).unwrap();
        assert_eq!(*driver.del_calls.borrow(), 1);
        assert_eq!(*driver.disable_calls.borrow(), 0);
        assert_eq!(*driver.refresh_calls.borrow(), 1);
    }

    #[test]
    fn owned_cleanup_disables_empty_engine_after_removal() {
        let driver = MockDriver {
            version: 1,
            enabled: true,
            rules: String::new(),
            ..MockDriver::default()
        };
        cleanup_owned_rules(&driver, &[rule("ours")]).unwrap();
        assert_eq!(*driver.del_calls.borrow(), 1);
        assert_eq!(*driver.disable_calls.borrow(), 1);
        assert_eq!(*driver.refresh_calls.borrow(), 1);
    }

    #[test]
    fn sar_aliases_resolve_to_real_partition_roots() {
        assert_eq!(
            resolve_target_path(Path::new("system/vendor/lib64/a.so")),
            Some(PathBuf::from("/vendor/lib64/a.so"))
        );
        assert_eq!(
            resolve_target_path(Path::new("system/product/app/A.apk")),
            Some(PathBuf::from("/product/app/A.apk"))
        );
    }

    #[test]
    fn replace_marker_rejects_module_preflight() {
        let temp = TempDir::new().unwrap();
        let module_root = temp.path().join("m");
        fs::create_dir_all(module_root.join("system/etc/demo")).unwrap();
        fs::write(module_root.join("system/etc/demo/.replace"), b"").unwrap();
        let module = Module {
            id: "m".to_string(),
            source_path: module_root,
            rules: ModuleRules {
                default_mode: MountMode::Overlay,
                paths: HashMap::new(),
            },
        };
        assert!(!module_preflight_supported(&module, &["system".to_string()]).unwrap());
    }
}
