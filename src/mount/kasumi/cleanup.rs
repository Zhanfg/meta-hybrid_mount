// Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-only

use anyhow::{Result, bail};

use super::common::feature_supported;
use crate::sys::kasumi::{
    self, KSM_FEATURE_CMDLINE_SPOOF, KSM_FEATURE_MAPS_SPOOF, KSM_FEATURE_MOUNT_HIDE,
    KSM_FEATURE_SELINUX_FIX, KSM_FEATURE_STATFS_SPOOF, KSM_FEATURE_UNAME_SPOOF, KasumiSpoofUname,
};

fn record_cleanup_error(errors: &mut Vec<String>, operation: &'static str, result: Result<()>) {
    if let Err(error) = result {
        errors.push(format!("{operation}: {error:#}"));
    }
}

fn should_attempt_feature(features: Option<i32>, required_feature: i32) -> bool {
    features.is_none_or(|bits| feature_supported(bits, required_feature))
}

/// Return an operable Kasumi runtime to a disabled, unspoofed baseline.
///
/// Every cleanup operation is attempted even if an earlier one fails. When
/// feature discovery itself fails, optional cleanup calls are still attempted
/// so a stale global spoof state is not silently left active.
pub fn rollback_runtime() -> Result<()> {
    let mut errors = Vec::new();
    let features = match kasumi::get_features() {
        Ok(features) => Some(features),
        Err(error) => {
            errors.push(format!("query features: {error:#}"));
            None
        }
    };

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

    if should_attempt_feature(features, KSM_FEATURE_MOUNT_HIDE) {
        record_cleanup_error(
            &mut errors,
            "disable mount hide",
            kasumi::set_mount_hide(false),
        );
    }
    if should_attempt_feature(features, KSM_FEATURE_MAPS_SPOOF) {
        record_cleanup_error(
            &mut errors,
            "disable maps spoof",
            kasumi::set_maps_spoof(false),
        );
    }
    if should_attempt_feature(features, KSM_FEATURE_STATFS_SPOOF) {
        record_cleanup_error(
            &mut errors,
            "disable statfs spoof",
            kasumi::set_statfs_spoof(false),
        );
    }
    if should_attempt_feature(features, KSM_FEATURE_SELINUX_FIX) {
        record_cleanup_error(
            &mut errors,
            "disable SELinux fix",
            kasumi::set_selinux_fix(false),
        );
    }
    if should_attempt_feature(features, KSM_FEATURE_CMDLINE_SPOOF) {
        record_cleanup_error(
            &mut errors,
            "clear cmdline spoof",
            kasumi::set_cmdline_str(""),
        );
    }
    if should_attempt_feature(features, KSM_FEATURE_UNAME_SPOOF) {
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
        bail!("Kasumi runtime cleanup incomplete: {}", errors.join(" | "));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::should_attempt_feature;
    use crate::sys::kasumi::KSM_FEATURE_MOUNT_HIDE;

    #[test]
    fn unknown_features_attempt_cleanup_conservatively() {
        assert!(should_attempt_feature(None, KSM_FEATURE_MOUNT_HIDE));
    }

    #[test]
    fn known_features_only_attempt_supported_cleanup() {
        assert!(should_attempt_feature(
            Some(KSM_FEATURE_MOUNT_HIDE),
            KSM_FEATURE_MOUNT_HIDE
        ));
        assert!(!should_attempt_feature(Some(0), KSM_FEATURE_MOUNT_HIDE));
    }
}
