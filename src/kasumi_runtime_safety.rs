// Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-only

use std::{
    fs,
    os::unix::fs::MetadataExt,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Deserializer, de::Error as _};

const DATA_ADB_ROOT: &str = "/data/adb";
const MANAGED_ROOTS: &[&str] = &[
    "/system",
    "/vendor",
    "/odm",
    "/product",
    "/system_ext",
    "/my_carrier",
];
const MAX_DIRECTORY_ENTRIES: usize = 100_000;
const MAX_DIRECTORY_DEPTH: usize = 64;

#[derive(Clone, Copy)]
enum RequiredLeaf {
    File,
    Directory,
}

fn ensure_root_owned_nonwritable(path: &Path, metadata: &fs::Metadata) -> Result<()> {
    if metadata.uid() != 0 {
        bail!("Kasumi runtime path is not root-owned: {}", path.display());
    }
    if metadata.mode() & 0o022 != 0 {
        bail!(
            "Kasumi runtime path is group/world writable: {} mode={:o}",
            path.display(),
            metadata.mode() & 0o7777
        );
    }
    Ok(())
}

fn ensure_real_path_component(path: &Path, metadata: &fs::Metadata) -> Result<()> {
    if metadata.file_type().is_symlink() {
        bail!(
            "Kasumi runtime path contains a symlink component: {}",
            path.display()
        );
    }
    ensure_root_owned_nonwritable(path, metadata)
}

fn ensure_trusted_existing_chain(
    anchor: &Path,
    path: &Path,
    leaf: Option<RequiredLeaf>,
) -> Result<()> {
    if !path.starts_with(anchor) {
        bail!(
            "Kasumi runtime path escaped trusted anchor: path={}, anchor={}",
            path.display(),
            anchor.display()
        );
    }

    let anchor_metadata = fs::symlink_metadata(anchor).with_context(|| {
        format!(
            "failed to inspect trusted Kasumi anchor {}",
            anchor.display()
        )
    })?;
    if !anchor_metadata.file_type().is_dir() {
        bail!(
            "trusted Kasumi anchor is not a directory: {}",
            anchor.display()
        );
    }
    ensure_real_path_component(anchor, &anchor_metadata)?;
    let canonical_anchor = fs::canonicalize(anchor).with_context(|| {
        format!(
            "failed to resolve trusted Kasumi anchor {}",
            anchor.display()
        )
    })?;
    if canonical_anchor != anchor {
        bail!(
            "trusted Kasumi anchor resolves elsewhere: {} -> {}",
            anchor.display(),
            canonical_anchor.display()
        );
    }

    let relative = path.strip_prefix(anchor).expect("path prefix checked");
    let mut cursor = anchor.to_path_buf();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            bail!(
                "Kasumi runtime path contains an invalid component: {}",
                path.display()
            );
        };
        cursor.push(name);
        let metadata = fs::symlink_metadata(&cursor).with_context(|| {
            format!("failed to inspect Kasumi runtime path {}", cursor.display())
        })?;
        ensure_real_path_component(&cursor, &metadata)?;
    }

    if let Some(required) = leaf {
        let metadata = fs::symlink_metadata(path)
            .with_context(|| format!("failed to inspect Kasumi source {}", path.display()))?;
        match required {
            RequiredLeaf::File if !metadata.file_type().is_file() => {
                bail!(
                    "Kasumi source is not a real regular file: {}",
                    path.display()
                )
            }
            RequiredLeaf::Directory if !metadata.file_type().is_dir() => {
                bail!("Kasumi source is not a real directory: {}", path.display())
            }
            _ => {}
        }
    }
    Ok(())
}

fn ensure_trusted_target_chain(anchor: &Path, target: &Path) -> Result<()> {
    if !target.starts_with(anchor) {
        bail!(
            "Kasumi target escaped trusted anchor: target={}, anchor={}",
            target.display(),
            anchor.display()
        );
    }

    let mut cursor = target;
    loop {
        match fs::symlink_metadata(cursor) {
            Ok(_) => return ensure_trusted_existing_chain(anchor, cursor, None),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to inspect Kasumi target {}", cursor.display())
                });
            }
        }
        let Some(parent) = cursor.parent() else {
            bail!(
                "Kasumi target has no trusted existing ancestor: {}",
                target.display()
            );
        };
        if parent == cursor || !parent.starts_with(anchor) {
            bail!(
                "Kasumi target has no trusted existing ancestor: {}",
                target.display()
            );
        }
        cursor = parent;
    }
}

fn is_managed_target(path: &Path) -> bool {
    MANAGED_ROOTS.iter().any(|root| path.starts_with(root))
}

fn mirror_anchor(path: &Path) -> Option<PathBuf> {
    let mut components = path.components();
    if components.next() != Some(Component::RootDir) {
        return None;
    }
    if components.next()?.as_os_str() != "dev" {
        return None;
    }
    let mirror = components.next()?.as_os_str().to_str()?;
    if !mirror.starts_with("kasumi_mirror") {
        return None;
    }
    Some(Path::new("/dev").join(mirror))
}

pub fn ensure_stable_target(path: &Path) -> Result<()> {
    crate::path_safety::ensure_kasumi_target_allowed(path)?;

    if is_managed_target(path) {
        return Ok(());
    }
    let data_adb = Path::new(DATA_ADB_ROOT);
    if path.starts_with(data_adb) {
        return ensure_trusted_target_chain(data_adb, path);
    }

    bail!(
        "stable Kasumi mutations only allow managed system partitions or {}: {}",
        DATA_ADB_ROOT,
        path.display()
    )
}

fn ensure_stable_file_source(path: &Path) -> Result<()> {
    let data_adb = Path::new(DATA_ADB_ROOT);
    if path.starts_with(data_adb) {
        return ensure_trusted_existing_chain(data_adb, path, Some(RequiredLeaf::File));
    }
    if let Some(anchor) = mirror_anchor(path) {
        return ensure_trusted_existing_chain(&anchor, path, Some(RequiredLeaf::File));
    }
    bail!(
        "direct Kasumi file sources must live under {} or /dev/kasumi_mirror*: {}",
        DATA_ADB_ROOT,
        path.display()
    )
}

fn ensure_stable_directory_source(path: &Path) -> Result<()> {
    let data_adb = Path::new(DATA_ADB_ROOT);
    let anchor = if path.starts_with(data_adb) {
        data_adb.to_path_buf()
    } else if let Some(anchor) = mirror_anchor(path) {
        anchor
    } else {
        bail!(
            "direct Kasumi directory sources must live under {} or /dev/kasumi_mirror*: {}",
            DATA_ADB_ROOT,
            path.display()
        );
    };

    ensure_trusted_existing_chain(&anchor, path, Some(RequiredLeaf::Directory))?;

    let mut queue = std::collections::VecDeque::from([(path.to_path_buf(), 0usize)]);
    let mut scanned = 0usize;
    while let Some((directory, depth)) = queue.pop_front() {
        if depth > MAX_DIRECTORY_DEPTH {
            bail!(
                "Kasumi source tree exceeds maximum depth at {}",
                directory.display()
            );
        }
        for entry in fs::read_dir(&directory)
            .with_context(|| format!("failed to scan Kasumi source tree {}", directory.display()))?
        {
            scanned += 1;
            if scanned > MAX_DIRECTORY_ENTRIES {
                bail!("Kasumi source tree exceeds maximum entry count ({MAX_DIRECTORY_ENTRIES})");
            }
            let entry = entry?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).with_context(|| {
                format!("failed to inspect Kasumi source entry {}", path.display())
            })?;
            ensure_real_path_component(&path, &metadata)?;
            if metadata.file_type().is_dir() {
                queue.push_back((path, depth + 1));
            } else if !metadata.file_type().is_file() {
                bail!(
                    "Kasumi source tree contains a special node: {}",
                    path.display()
                );
            }
        }
    }
    Ok(())
}

pub fn validate_live_config(config: &crate::conf::schema::Config) -> Result<()> {
    if !config.kasumi.enabled {
        return Ok(());
    }
    for rule in &config.kasumi.kstat_rules {
        ensure_stable_target(&rule.target_pathname).context("unsafe Kasumi kstat target")?;
    }
    if !config.kasumi.statfs_spoof.path.as_os_str().is_empty() {
        ensure_stable_target(&config.kasumi.statfs_spoof.path)
            .context("unsafe Kasumi statfs target")?;
    }
    Ok(())
}

pub fn deserialize_stable_target<'de, D>(deserializer: D) -> Result<PathBuf, D::Error>
where
    D: Deserializer<'de>,
{
    let path = PathBuf::deserialize(deserializer)?;
    ensure_stable_target(&path).map_err(D::Error::custom)?;
    Ok(path)
}

pub fn deserialize_stable_file_source<'de, D>(deserializer: D) -> Result<PathBuf, D::Error>
where
    D: Deserializer<'de>,
{
    let path = PathBuf::deserialize(deserializer)?;
    ensure_stable_file_source(&path).map_err(D::Error::custom)?;
    Ok(path)
}

pub fn deserialize_stable_directory_source<'de, D>(deserializer: D) -> Result<PathBuf, D::Error>
where
    D: Deserializer<'de>,
{
    let path = PathBuf::deserialize(deserializer)?;
    ensure_stable_directory_source(&path).map_err(D::Error::custom)?;
    Ok(path)
}

pub fn deserialize_regular_file_type<'de, D>(deserializer: D) -> Result<i32, D::Error>
where
    D: Deserializer<'de>,
{
    let file_type = i32::deserialize(deserializer)?;
    if file_type == libc::DT_REG as i32 {
        Ok(file_type)
    } else {
        Err(D::Error::custom(format!(
            "direct Kasumi ADD only accepts regular-file dirent type; got {file_type}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn managed_targets_are_classified_as_stable_roots() {
        assert!(is_managed_target(Path::new("/system/app/Example.apk")));
        assert!(is_managed_target(Path::new(
            "/vendor/lib64/soundfx/libfx.so"
        )));
        assert!(!is_managed_target(Path::new("/data/local/tmp/file")));
    }

    #[test]
    fn mirror_anchor_only_accepts_direct_kasumi_mirror_namespace() {
        assert_eq!(
            mirror_anchor(Path::new("/dev/kasumi_mirror_123/system/app/file")),
            Some(PathBuf::from("/dev/kasumi_mirror_123"))
        );
        assert!(mirror_anchor(Path::new("/dev/socket/file")).is_none());
        assert!(mirror_anchor(Path::new("/data/adb/kasumi_mirror/file")).is_none());
    }

    #[test]
    fn generic_trusted_chain_rejects_writable_parent() {
        let temp = TempDir::new().unwrap();
        let anchor = temp.path().join("anchor");
        let parent = anchor.join("parent");
        fs::create_dir_all(&parent).unwrap();
        fs::write(parent.join("file"), b"ok").unwrap();
        fs::set_permissions(&anchor, fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o777)).unwrap();

        assert!(
            ensure_trusted_existing_chain(&anchor, &parent.join("file"), Some(RequiredLeaf::File))
                .is_err()
        );
    }
}
