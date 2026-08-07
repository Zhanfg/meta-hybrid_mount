// Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-only

use std::{
    collections::VecDeque,
    fs,
    os::unix::fs::{FileTypeExt, MetadataExt},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};

use crate::domain::{ModuleRules, MountMode};

const MAX_SCANNED_ENTRIES: usize = 100_000;
const MAX_DIRECTORY_DEPTH: usize = 64;

pub(super) fn first_blocked_special_node(
    module_root: &Path,
    rules: &ModuleRules,
) -> Result<Option<PathBuf>> {
    let canonical_module_root = fs::canonicalize(module_root)
        .with_context(|| format!("failed to resolve module root {}", module_root.display()))?;
    let mut queue = VecDeque::from([(module_root.to_path_buf(), PathBuf::new(), 0usize)]);
    let mut scanned_entries = 0usize;

    while let Some((directory, relative_directory, depth)) = queue.pop_front() {
        if depth > MAX_DIRECTORY_DEPTH {
            bail!(
                "module tree exceeds special-node scan depth at {}",
                relative_directory.display()
            );
        }

        let entries = fs::read_dir(&directory)
            .with_context(|| format!("failed to inspect module path {}", directory.display()))?;
        for entry in entries {
            scanned_entries += 1;
            if scanned_entries > MAX_SCANNED_ENTRIES {
                bail!("module tree exceeds special-node scan entry limit ({MAX_SCANNED_ENTRIES})");
            }

            let entry = entry.with_context(|| {
                format!("failed to enumerate module path {}", directory.display())
            })?;
            let relative_path = relative_directory.join(entry.file_name());
            let effective_mode = rules.effective_mode(&relative_path);
            let file_type = entry
                .file_type()
                .with_context(|| format!("failed to inspect {}", entry.path().display()))?;

            if matches!(effective_mode, MountMode::Ignore) {
                continue;
            }

            if file_type.is_dir() {
                queue.push_back((entry.path(), relative_path, depth + 1));
                continue;
            }

            if file_type.is_file() {
                continue;
            }

            if file_type.is_symlink() {
                // OverlayFS/Magic preserve a payload symlink as a symlink. Kasumi's
                // kernel ADD_RULE, however, resolves its source with LOOKUP_FOLLOW.
                // Therefore a Kasumi symlink is safe only when it resolves to a
                // regular file inside the same module tree.
                if matches!(effective_mode, MountMode::Kasumi) {
                    let resolved = match fs::canonicalize(entry.path()) {
                        Ok(resolved) => resolved,
                        Err(_) => return Ok(Some(relative_path)),
                    };
                    if !resolved.starts_with(&canonical_module_root) || !resolved.is_file() {
                        return Ok(Some(relative_path));
                    }
                }
                continue;
            }

            // A 0:0 character device is the established module whiteout
            // representation. Kasumi compiles it into a hide rule rather than
            // exposing a device node, so it remains a supported safe marker.
            if file_type.is_char_device() {
                let metadata = fs::symlink_metadata(entry.path()).with_context(|| {
                    format!("failed to inspect special node {}", entry.path().display())
                })?;
                if metadata.rdev() == 0 {
                    continue;
                }
            }

            // Any remaining filesystem node type is rejected by default.
            // This covers block/character devices, FIFOs, sockets and future
            // special types without depending on a finite allow-deny list.
            return Ok(Some(relative_path));
        }
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use std::os::unix::{fs::symlink, net::UnixListener};

    use tempfile::TempDir;

    use super::*;

    fn default_rules() -> ModuleRules {
        ModuleRules {
            default_mode: MountMode::Overlay,
            ..Default::default()
        }
    }

    fn kasumi_rules() -> ModuleRules {
        ModuleRules {
            default_mode: MountMode::Kasumi,
            ..Default::default()
        }
    }

    #[test]
    fn regular_files_directories_and_overlay_symlinks_are_allowed() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join("system/app")).unwrap();
        fs::write(temp.path().join("system/app/example.apk"), b"ok").unwrap();
        symlink(
            temp.path().join("system/app/example.apk"),
            temp.path().join("system/app/example-link"),
        )
        .unwrap();

        assert!(
            first_blocked_special_node(temp.path(), &default_rules())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn kasumi_allows_symlink_to_regular_file_inside_module() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join("system/app")).unwrap();
        fs::write(temp.path().join("system/app/example.apk"), b"ok").unwrap();
        symlink(
            "example.apk",
            temp.path().join("system/app/example-link"),
        )
        .unwrap();

        assert!(
            first_blocked_special_node(temp.path(), &kasumi_rules())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn kasumi_rejects_symlink_that_escapes_module_tree() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join("system/etc")).unwrap();
        symlink("/dev/null", temp.path().join("system/etc/escape")).unwrap();

        assert_eq!(
            first_blocked_special_node(temp.path(), &kasumi_rules()).unwrap(),
            Some(PathBuf::from("system/etc/escape"))
        );
    }

    #[test]
    fn kasumi_rejects_broken_symlink_source() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join("system/etc")).unwrap();
        symlink("missing", temp.path().join("system/etc/broken")).unwrap();

        assert_eq!(
            first_blocked_special_node(temp.path(), &kasumi_rules()).unwrap(),
            Some(PathBuf::from("system/etc/broken"))
        );
    }

    #[test]
    fn unix_socket_payload_is_blocked() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join("system/etc")).unwrap();
        let socket_path = temp.path().join("system/etc/runtime.sock");
        let _listener = UnixListener::bind(&socket_path).unwrap();

        assert_eq!(
            first_blocked_special_node(temp.path(), &default_rules()).unwrap(),
            Some(PathBuf::from("system/etc/runtime.sock"))
        );
    }

    #[test]
    fn ignored_special_subtree_does_not_quarantine_module() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join("system/etc/private")).unwrap();
        let socket_path = temp.path().join("system/etc/private/runtime.sock");
        let _listener = UnixListener::bind(&socket_path).unwrap();
        let mut rules = default_rules();
        rules
            .paths
            .insert("system/etc/private".to_string(), MountMode::Ignore);

        assert!(
            first_blocked_special_node(temp.path(), &rules)
                .unwrap()
                .is_none()
        );
    }
}
