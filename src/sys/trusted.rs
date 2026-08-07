// Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-only

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Take},
    path::Path,
};

use anyhow::{Context, Result, bail};

fn require_private_path(path: &Path) -> Result<()> {
    if !path.is_absolute() || !path.starts_with("/data/adb") {
        bail!("trusted runtime path must stay under /data/adb: {}", path.display());
    }
    Ok(())
}

fn validate_private_parent(path: &Path) -> Result<()> {
    require_private_path(path)?;
    let parent = path
        .parent()
        .with_context(|| format!("trusted path has no parent: {}", path.display()))?;
    let canonical_parent = fs::canonicalize(parent)
        .with_context(|| format!("failed to canonicalize trusted parent {}", parent.display()))?;
    if canonical_parent != parent {
        bail!(
            "trusted parent resolves through a different path: {} -> {}",
            parent.display(),
            canonical_parent.display()
        );
    }

    let metadata = fs::symlink_metadata(parent)
        .with_context(|| format!("failed to inspect trusted parent {}", parent.display()))?;
    if !metadata.file_type().is_dir() {
        bail!("trusted parent is not a real directory: {}", parent.display());
    }
    #[cfg(unix)]
    {
        if metadata.uid() != 0 {
            bail!("trusted parent is not owned by root: {}", parent.display());
        }
        if metadata.mode() & 0o022 != 0 {
            bail!(
                "trusted parent is group/world writable: {} mode={:o}",
                parent.display(),
                metadata.mode() & 0o7777
            );
        }
    }
    Ok(())
}

pub(crate) fn ensure_private_dir(path: &Path, mode: u32) -> Result<()> {
    require_private_path(path)?;
    validate_private_parent(path)?;

    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.file_type().is_dir() {
                bail!("trusted runtime directory is not a real directory: {}", path.display());
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(path)
                .with_context(|| format!("failed to create trusted runtime directory {}", path.display()))?;
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to inspect trusted runtime directory {}", path.display()));
        }
    }

    let canonical = fs::canonicalize(path)
        .with_context(|| format!("failed to canonicalize trusted directory {}", path.display()))?;
    if canonical != path {
        bail!(
            "trusted runtime directory resolves through a different path: {} -> {}",
            path.display(),
            canonical.display()
        );
    }

    #[cfg(unix)]
    {
        let metadata = fs::symlink_metadata(path)
            .with_context(|| format!("failed to inspect trusted directory {}", path.display()))?;
        if metadata.uid() != 0 {
            bail!("trusted runtime directory is not owned by root: {}", path.display());
        }
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).with_context(|| {
            format!("failed to secure trusted runtime directory {}", path.display())
        })?;
        let secured = fs::symlink_metadata(path)
            .with_context(|| format!("failed to re-check trusted directory {}", path.display()))?;
        if secured.uid() != 0 || secured.mode() & 0o777 != mode {
            bail!(
                "trusted runtime directory permissions did not stick: {} uid={} mode={:o}",
                path.display(),
                secured.uid(),
                secured.mode() & 0o7777
            );
        }
    }

    Ok(())
}

pub(crate) fn open_private_regular(path: &Path, max_bytes: u64) -> Result<File> {
    validate_private_parent(path)?;

    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let file = options
        .open(path)
        .with_context(|| format!("failed to open trusted private file {}", path.display()))?;
    let metadata = file
        .metadata()
        .with_context(|| format!("failed to inspect trusted private file {}", path.display()))?;
    if !metadata.file_type().is_file() {
        bail!("trusted private path is not a regular file: {}", path.display());
    }
    if metadata.len() > max_bytes {
        bail!(
            "trusted private file exceeds size limit: {} size={} limit={}",
            path.display(),
            metadata.len(),
            max_bytes
        );
    }
    #[cfg(unix)]
    {
        if metadata.uid() != 0 {
            bail!("trusted private file is not owned by root: {}", path.display());
        }
        if metadata.mode() & 0o022 != 0 {
            bail!(
                "trusted private file is group/world writable: {} mode={:o}",
                path.display(),
                metadata.mode() & 0o7777
            );
        }
        if metadata.nlink() != 1 {
            bail!(
                "trusted private file has unexpected hard-link count: {} links={}",
                path.display(),
                metadata.nlink()
            );
        }
    }
    Ok(file)
}

pub(crate) fn read_private_text(path: &Path, max_bytes: u64) -> Result<String> {
    let file = open_private_regular(path, max_bytes)?;
    let mut content = String::new();
    let mut limited: Take<File> = file.take(max_bytes + 1);
    limited
        .read_to_string(&mut content)
        .with_context(|| format!("failed to read trusted UTF-8 file {}", path.display()))?;
    if content.len() as u64 > max_bytes {
        bail!("trusted private file grew beyond size limit: {}", path.display());
    }
    Ok(content)
}

pub(crate) fn validate_private_regular(path: &Path, max_bytes: u64) -> Result<()> {
    let _ = open_private_regular(path, max_bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::symlink};

    use super::*;

    #[test]
    fn rejects_paths_outside_android_private_root() {
        assert!(open_private_regular(Path::new("/tmp/file"), 1024).is_err());
        assert!(ensure_private_dir(Path::new("/tmp/run"), 0o700).is_err());
    }

    #[test]
    fn private_file_reader_rejects_final_symlink() {
        let root = Path::new("/data/adb/rehybird-trusted-test");
        let _ = fs::remove_dir_all(root);
        fs::create_dir_all(root).unwrap();
        fs::set_permissions(root, fs::Permissions::from_mode(0o700)).unwrap();
        let target = root.join("target");
        let link = root.join("link");
        fs::write(&target, b"ok").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
        symlink(&target, &link).unwrap();
        assert!(open_private_regular(&link, 1024).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
