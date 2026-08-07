// Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-only

use std::fs;

use anyhow::{Context, Result, bail};

use crate::conf::schema::KasumiConfig;

const KERNEL_RELEASE_PATH: &str = "/proc/sys/kernel/osrelease";

pub(crate) fn validate_override(config: &KasumiConfig) -> Result<()> {
    let requested = config.lkm_kmi_override.trim();
    if requested.is_empty() {
        return Ok(());
    }

    let release = fs::read_to_string(KERNEL_RELEASE_PATH)
        .with_context(|| format!("failed to read {KERNEL_RELEASE_PATH}"))?;
    validate_override_for_release(requested, release.trim())
}

fn validate_override_for_release(requested: &str, release: &str) -> Result<()> {
    let requested = parse_override(requested)?;
    let detected = parse_detected_kmi(release).with_context(|| {
        format!(
            "cannot verify Kasumi KMI override {} against kernel release {}; refusing LKM autoload",
            requested.canonical, release
        )
    })?;

    if requested.canonical != detected.canonical {
        bail!(
            "Kasumi KMI override mismatch: configured={}, detected={}, kernel_release={}; refusing LKM autoload",
            requested.canonical,
            detected.canonical,
            release
        );
    }

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedKmi {
    canonical: String,
}

fn parse_override(value: &str) -> Result<ParsedKmi> {
    let value = value.trim();
    let remainder = value
        .strip_prefix("android")
        .context("Kasumi KMI override must start with 'android'")?;
    let (android_version, kernel_version) = remainder
        .split_once('-')
        .context("Kasumi KMI override must use androidN-M.m format")?;

    if android_version.is_empty() || !android_version.bytes().all(|byte| byte.is_ascii_digit()) {
        bail!("Kasumi KMI override has an invalid Android version: {value}");
    }
    validate_major_minor(kernel_version)
        .with_context(|| format!("Kasumi KMI override has an invalid kernel version: {value}"))?;

    Ok(ParsedKmi {
        canonical: format!("android{android_version}-{kernel_version}"),
    })
}

fn parse_detected_kmi(release: &str) -> Result<ParsedKmi> {
    let release = release.trim();
    if release.is_empty() {
        bail!("kernel release is empty");
    }

    let major_minor = release_major_minor(release)?;
    let android_marker = release
        .find("-android")
        .context("kernel release does not expose an Android KMI marker")?;
    let version_start = android_marker + "-android".len();
    let version_end = release[version_start..]
        .find('-')
        .map(|offset| version_start + offset)
        .unwrap_or(release.len());
    let android_version = &release[version_start..version_end];

    if android_version.is_empty() || !android_version.bytes().all(|byte| byte.is_ascii_digit()) {
        bail!("kernel release has an invalid Android KMI version");
    }

    Ok(ParsedKmi {
        canonical: format!("android{android_version}-{major_minor}"),
    })
}

fn release_major_minor(release: &str) -> Result<String> {
    let mut components = release.split('.');
    let major = components.next().context("kernel release has no major version")?;
    let minor = components.next().context("kernel release has no minor version")?;
    validate_numeric_component(major, "major")?;
    validate_numeric_component(minor, "minor")?;
    Ok(format!("{major}.{minor}"))
}

fn validate_major_minor(value: &str) -> Result<()> {
    let mut components = value.split('.');
    let major = components.next().context("missing major version")?;
    let minor = components.next().context("missing minor version")?;
    if components.next().is_some() {
        bail!("kernel version contains more than major.minor");
    }
    validate_numeric_component(major, "major")?;
    validate_numeric_component(minor, "minor")
}

fn validate_numeric_component(value: &str, label: &str) -> Result<()> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        bail!("{label} version is not numeric");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_override_needs_no_kernel_probe() {
        let config = KasumiConfig {
            lkm_kmi_override: String::new(),
            ..KasumiConfig::default()
        };

        validate_override(&config).unwrap();
    }

    #[test]
    fn accepts_exact_detected_kmi() {
        validate_override_for_release(
            "android15-6.6",
            "6.6.89-android15-8-g123456789abc-ab12345678",
        )
        .unwrap();
    }

    #[test]
    fn rejects_android_generation_mismatch() {
        let error = validate_override_for_release(
            "android14-6.6",
            "6.6.89-android15-8-g123456789abc-ab12345678",
        )
        .unwrap_err();

        assert!(error.to_string().contains("configured=android14-6.6"));
        assert!(error.to_string().contains("detected=android15-6.6"));
    }

    #[test]
    fn rejects_kernel_major_minor_mismatch() {
        let error = validate_override_for_release(
            "android15-5.15",
            "6.6.89-android15-8-g123456789abc-ab12345678",
        )
        .unwrap_err();

        assert!(error.to_string().contains("configured=android15-5.15"));
        assert!(error.to_string().contains("detected=android15-6.6"));
    }

    #[test]
    fn rejects_unverifiable_vendor_release() {
        let error = validate_override_for_release("android15-6.6", "6.6.89-vendor-release")
            .unwrap_err();

        assert!(error.to_string().contains("cannot verify Kasumi KMI override"));
    }

    #[test]
    fn rejects_malformed_override() {
        assert!(parse_override("6.6").is_err());
        assert!(parse_override("android-6.6").is_err());
        assert!(parse_override("android15-6").is_err());
        assert!(parse_override("android15-6.6.1").is_err());
    }
}
