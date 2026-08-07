// Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-only

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Take},
    path::Path,
};

use anyhow::{Context, Result, bail};

#[cfg(feature = "control-plane")]
use crate::conf::cli::Cli;
use crate::{
    conf::{config::Config, schema::BlacklistConfig},
    defs,
};

const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
const MAX_BLACKLIST_BYTES: u64 = 256 * 1024;

fn open_without_following(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    options
        .open(path)
        .with_context(|| format!("failed to open trusted file {}", path.display()))
}

fn validate_private_android_path(path: &Path, metadata: &fs::Metadata) -> Result<()> {
    if !path.starts_with("/data/adb") {
        return Ok(());
    }

    let parent = path
        .parent()
        .with_context(|| format!("trusted file has no parent: {}", path.display()))?;
    let canonical_parent = fs::canonicalize(parent)
        .with_context(|| format!("failed to canonicalize trusted parent {}", parent.display()))?;
    if canonical_parent != parent {
        bail!(
            "trusted parent resolves through a different path: {} -> {}",
            parent.display(),
            canonical_parent.display()
        );
    }

    #[cfg(unix)]
    {
        if metadata.uid() != 0 {
            bail!("trusted file is not owned by root: {}", path.display());
        }
        if metadata.mode() & 0o022 != 0 {
            bail!(
                "trusted file is group/world writable: {} mode={:o}",
                path.display(),
                metadata.mode() & 0o7777
            );
        }
        if metadata.nlink() != 1 {
            bail!(
                "trusted file has unexpected hard-link count: {} links={}",
                path.display(),
                metadata.nlink()
            );
        }
    }

    Ok(())
}

fn read_trusted_text_file(path: &Path, max_bytes: u64) -> Result<String> {
    let file = open_without_following(path)?;
    let metadata = file
        .metadata()
        .with_context(|| format!("failed to inspect trusted file {}", path.display()))?;
    if !metadata.file_type().is_file() {
        bail!("trusted path is not a regular file: {}", path.display());
    }
    if metadata.len() > max_bytes {
        bail!(
            "trusted file exceeds size limit: {} size={} limit={}",
            path.display(),
            metadata.len(),
            max_bytes
        );
    }
    validate_private_android_path(path, &metadata)?;

    let mut content = String::new();
    let mut limited: Take<File> = file.take(max_bytes + 1);
    limited
        .read_to_string(&mut content)
        .with_context(|| format!("failed to read trusted UTF-8 file {}", path.display()))?;
    if content.len() as u64 > max_bytes {
        bail!(
            "trusted file grew beyond size limit while reading: {}",
            path.display()
        );
    }
    Ok(content)
}

fn load_main_config(path: &Path) -> Result<Config> {
    let content = read_trusted_text_file(path, MAX_CONFIG_BYTES)?;
    let mut config = toml::from_str::<Config>(&content)
        .with_context(|| format!("failed to parse config file {}", path.display()))?;
    config.sanitize_disabled_features();
    crate::path_safety::validate_config_targets(&config)
        .context("config contains a protected runtime or radio-critical target")?;
    Ok(config)
}

pub(crate) fn load_module_blacklist(mut config: Config) -> Result<Config> {
    let path = Path::new(defs::MODULE_BLACKLIST_FILE);
    let content = read_trusted_text_file(path, MAX_BLACKLIST_BYTES)?;
    let blacklist = toml::from_str::<BlacklistConfig>(&content)
        .with_context(|| format!("failed to parse blacklist file {}", path.display()))?;
    crate::scoped_log!(
        debug,
        "conf:loader",
        "blacklist loaded: path={}, entries={}",
        path.display(),
        blacklist.blacklist.len()
    );
    config.module_blacklist = blacklist.blacklist;

    Ok(config)
}

pub fn load_default_config() -> Result<Config> {
    let default_path = Path::new(defs::CONFIG_FILE);
    crate::scoped_log!(
        debug,
        "conf:loader",
        "start: mode=default, path={}",
        default_path.display()
    );
    let config = load_main_config(default_path).with_context(|| {
        format!(
            "Failed to load config from default path: {}",
            default_path.display()
        )
    })?;

    let config = load_module_blacklist(config)?;

    crate::scoped_log!(
        debug,
        "conf:loader",
        "complete: mode=default, path={}",
        default_path.display()
    );

    Ok(config)
}

#[cfg(feature = "control-plane")]
pub fn load_config(cli: &Cli) -> Result<Config> {
    let config_path = &cli.config;
    crate::scoped_log!(
        debug,
        "conf:loader",
        "start: path={}",
        config_path.display()
    );

    let config = load_main_config(config_path)
        .with_context(|| format!("Failed to load config from {}", config_path.display()))?;
    let config = load_module_blacklist(config)?;

    crate::scoped_log!(
        debug,
        "conf:loader",
        "complete: path={}",
        config_path.display()
    );

    Ok(config)
}

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::symlink};

    use super::*;

    #[test]
    fn trusted_reader_accepts_regular_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        fs::write(&path, "default_mode = \"overlay\"\n").unwrap();

        assert!(read_trusted_text_file(&path, 1024).is_ok());
    }

    #[test]
    fn trusted_reader_rejects_final_symlink() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target.toml");
        let link = temp.path().join("config.toml");
        fs::write(&target, "default_mode = \"overlay\"\n").unwrap();
        symlink(&target, &link).unwrap();

        assert!(read_trusted_text_file(&link, 1024).is_err());
    }

    #[test]
    fn trusted_reader_rejects_oversized_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        fs::write(&path, vec![b'x'; 1025]).unwrap();

        assert!(read_trusted_text_file(&path, 1024).is_err());
    }

    #[test]
    fn startup_loader_rejects_protected_custom_bind_target() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        fs::write(
            &path,
            r#"
moduledir = "/data/adb/modules"
mountsource = "KSU"
overlay_mode = "ext4"
disable_umount = false
default_mode = "overlay"

[[custom_mounts]]
source = "/data/local/tmp/source"
target = "/vendor/firmware/modem.mbn"
"#,
        )
        .unwrap();

        assert!(load_main_config(&path).is_err());
    }
}
