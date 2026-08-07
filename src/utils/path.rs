// Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-only

use std::{
    ffi::OsStr,
    fs, io,
    path::{Component, Path, PathBuf},
};

pub fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    let mut saw_root = false;

    for component in path.components() {
        match component {
            Component::RootDir => {
                normalized.push(Path::new("/"));
                saw_root = true;
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(
                    normalized.components().next_back(),
                    Some(Component::Normal(_))
                ) {
                    normalized.pop();
                } else if !saw_root {
                    normalized.push("..");
                }
            }
            Component::Normal(value) => normalized.push(value),
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
        }
    }

    if saw_root && normalized.as_os_str().is_empty() {
        PathBuf::from("/")
    } else {
        normalized
    }
}

pub fn path_file_name_eq(path: &Path, expected: &str) -> bool {
    path.file_name()
        .is_some_and(|name| name == OsStr::new(expected))
}

pub fn resolve_link_path(path: &Path) -> io::Result<PathBuf> {
    match fs::read_link(path) {
        Ok(target) if target.is_absolute() => Ok(normalize_path(&target)),
        Ok(target) => {
            let parent = path.parent().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "symlink path has no parent")
            })?;
            Ok(normalize_path(&parent.join(target)))
        }
        Err(err) if err.kind() == io::ErrorKind::InvalidInput => Ok(normalize_path(path)),
        Err(err) => Err(err),
    }
}

#[cfg(feature = "kasumi")]
fn path_entry_exists(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

#[cfg(feature = "kasumi")]
pub fn resolve_path_with_root(system_root: &Path, path: &Path) -> io::Result<PathBuf> {
    if !system_root.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("system root must be absolute: {}", system_root.display()),
        ));
    }

    let canonical_root = fs::canonicalize(system_root).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "failed to resolve system root {}: {error}",
                system_root.display()
            ),
        )
    })?;
    let virtual_path = if path.is_absolute() {
        normalize_path(path)
    } else {
        normalize_path(&Path::new("/").join(path))
    };
    let relative = virtual_path.strip_prefix("/").map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("virtual path must be absolute: {error}"),
        )
    })?;
    let translated_path = normalize_path(&canonical_root.join(relative));
    if !translated_path.starts_with(&canonical_root) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "translated path {} escaped system root {}",
                translated_path.display(),
                canonical_root.display()
            ),
        ));
    }

    let parent = translated_path.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "translated path has no parent")
    })?;
    let filename = translated_path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "translated path has no filename",
        )
    })?;

    let mut current = parent.to_path_buf();
    let mut missing_suffix = Vec::new();
    while current != canonical_root && !path_entry_exists(&current)? {
        let name = current.file_name().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("path has no component below root: {}", current.display()),
            )
        })?;
        missing_suffix.push(name.to_os_string());
        if !current.pop() {
            break;
        }
    }

    if !path_entry_exists(&current)? {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no existing ancestor for {}", translated_path.display()),
        ));
    }

    let mut resolved = fs::canonicalize(&current).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "failed to resolve existing ancestor {}: {error}",
                current.display()
            ),
        )
    })?;
    if canonical_root != Path::new("/") && !resolved.starts_with(&canonical_root) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "resolved ancestor {} escaped system root {}",
                resolved.display(),
                canonical_root.display()
            ),
        ));
    }

    for component in missing_suffix.iter().rev() {
        resolved.push(component);
    }
    resolved.push(filename);
    let resolved = normalize_path(&resolved);

    if canonical_root == Path::new("/") {
        return Ok(resolved);
    }

    let relative = resolved.strip_prefix(&canonical_root).map_err(|error| {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "resolved path {} escaped system root {}: {error}",
                resolved.display(),
                canonical_root.display()
            ),
        )
    })?;
    Ok(normalize_path(&Path::new("/").join(relative)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_path_preserves_leading_relative_parents() {
        assert_eq!(
            normalize_path(Path::new("../../system/bin")),
            PathBuf::from("../../system/bin")
        );
    }

    #[test]
    fn normalize_path_clamps_absolute_parents_at_root() {
        assert_eq!(
            normalize_path(Path::new("/../../system")),
            PathBuf::from("/system")
        );
    }

    #[test]
    fn normalize_path_removes_resolved_parent_components() {
        assert_eq!(
            normalize_path(Path::new("system/../vendor")),
            PathBuf::from("vendor")
        );
    }

    #[cfg(all(feature = "kasumi", unix))]
    #[test]
    fn rooted_resolution_follows_parent_symlinks_and_preserves_filename() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        fs::create_dir_all(root.join("system")).unwrap();
        fs::create_dir_all(root.join("vendor")).unwrap();
        symlink("../vendor", root.join("system/link")).unwrap();

        assert_eq!(
            resolve_path_with_root(&root, Path::new("/system/link/file")).unwrap(),
            PathBuf::from("/vendor/file")
        );
    }

    #[cfg(all(feature = "kasumi", unix))]
    #[test]
    fn rooted_resolution_rejects_parent_symlink_escape() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        let outside = temp.path().join("outside");
        fs::create_dir_all(root.join("system")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        symlink(&outside, root.join("system/link")).unwrap();

        assert!(resolve_path_with_root(&root, Path::new("/system/link/file")).is_err());
    }

    #[cfg(feature = "kasumi")]
    #[test]
    fn rooted_resolution_preserves_missing_parent_components() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        fs::create_dir_all(root.join("system")).unwrap();

        assert_eq!(
            resolve_path_with_root(&root, Path::new("/system/missing/sub/file")).unwrap(),
            PathBuf::from("/system/missing/sub/file")
        );
    }
}
