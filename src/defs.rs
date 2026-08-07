// Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-only

#[cfg(any(target_os = "linux", target_os = "android"))]
#[allow(dead_code)]
pub const HYBRID_MOUNT_DIR: &str = "/data/adb/hybrid-mount";
pub const MODULES_DIR: &str = "/data/adb/modules";
#[allow(dead_code)]
pub const HYBRID_MOUNT_MODULE_DIR: &str = "/data/adb/modules/hybrid_mount";

pub const MODULES_IMG_FILE: &str = "/data/adb/hybrid-mount/modules.img";
#[allow(dead_code)]
pub const KASUMI_IMG_FILE: &str = "/data/adb/hybrid-mount/kasumi.img";
pub const RUN_DIR: &str = "/data/adb/hybrid-mount/run/";
pub const STATE_FILE: &str = "/data/adb/hybrid-mount/run/daemon_state.json";
pub const SOCKET_FILE: &str = "/data/adb/hybrid-mount/run/daemon.sock";
pub const PID_FILE: &str = "/data/adb/hybrid-mount/run/daemon.pid";
pub const KASUMI_LKM_OWNER_FILE: &str = "/data/adb/hybrid-mount/run/kasumi_lkm_owner.json";
pub const KASUMI_RULE_SNAPSHOT_FILE: &str = "/data/adb/hybrid-mount/run/kasumi_mount_rules.json";
pub const SYSTEM_RW_DIR: &str = "/data/adb/hybrid-mount/rw";
pub const CONFIG_FILE: &str = "/data/adb/hybrid-mount/config.toml";
pub const MODULE_BLACKLIST_FILE: &str = "/data/adb/hybrid-mount/module_blacklist.toml";
pub const USER_HIDE_RULES_FILE: &str = "/data/adb/hybrid-mount/user_hide_rules.json";
pub const MODULE_PROP_FILE: &str = "/data/adb/modules/hybrid_mount/module.prop";
pub const KASUMI_MIRROR_DIR: &str = "/dev/kasumi_mirror";
pub const KASUMI_LKM_DIR: &str = "/data/adb/modules/hybrid_mount/kasumi_lkm";
pub const KASUMI_LKM_MODULE_NAME: &str = "kasumi_lkm";

pub const DISABLE_FILE_NAME: &str = "disable";
pub const REMOVE_FILE_NAME: &str = "remove";
pub const SKIP_MOUNT_FILE_NAME: &str = "skip_mount";
pub const REPLACE_DIR_FILE_NAME: &str = ".replace";
#[cfg(any(target_os = "linux", target_os = "android"))]
pub const REPLACE_DIR_XATTR: &str = "trusted.overlay.opaque";

/// Mount roots that must remain visible in KernelSU child namespaces.
///
/// Detaching these library trees after zygote or vendor services have mapped
/// shared objects can crash consumers mid-flight. Bluetooth, radio, graphics,
/// and integrity components commonly resolve dependencies from these roots.
pub const IGNORE_UNMOUNT_PARTITIONS: &[&str] = &[
    "/vendor/lib",
    "/vendor/lib64",
    "/system/lib",
    "/system/lib64",
];

pub const MANAGED_PARTITIONS: &[&str] = &[
    "odm",
    "product",
    "system_ext",
    "vendor",
    "apex",
    "mi_ext",
    "my_bigball",
    "my_carrier",
    "my_company",
    "my_engineering",
    "my_heytap",
    "my_manifest",
    "my_preload",
    "my_product",
    "my_region",
    "my_reserve",
    "my_stock",
    "oem",
    "optics",
    "prism",
];

pub const MAX_MERGE_JSON_DEPTH: usize = 64;

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn writable_images_stay_inside_private_data_directory() {
        for image in [MODULES_IMG_FILE, KASUMI_IMG_FILE] {
            let path = Path::new(image);
            assert!(path.starts_with(HYBRID_MOUNT_DIR), "{image}");
            assert!(!path.starts_with("/dev"), "{image}");
            assert!(!path.starts_with("/sys"), "{image}");
            assert!(!path.starts_with("/proc"), "{image}");
        }
    }
}
