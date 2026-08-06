// Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-only

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};
#[cfg(any(target_os = "linux", target_os = "android"))]
use std::{ffi::OsString, os::unix::ffi::OsStringExt};

use anyhow::{Context, Result, bail};
#[cfg(any(target_os = "linux", target_os = "android"))]
use rustix::{
    io::Errno,
    mount::{UnmountFlags, unmount},
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct MountEntry {
    id: u64,
    mount_point: PathBuf,
}

/// Tracks mounts created during one boot-time mount transaction.
///
/// The baseline is captured before storage preparation. If any controller
/// stage returns early, `Drop` detaches every newly-created in-scope mount in
/// reverse mount-id order. A successful finalization explicitly disarms it.
pub struct MountTransaction {
    baseline_ids: HashSet<u64>,
    scope_roots: Vec<PathBuf>,
    armed: bool,
}

impl MountTransaction {
    pub fn begin<I, P>(scope_roots: I) -> Result<Self>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        let baseline_ids = read_mountinfo()?
            .into_iter()
            .map(|entry| entry.id)
            .collect();
        let mut transaction = Self {
            baseline_ids,
            scope_roots: Vec::new(),
            armed: true,
        };
        for root in scope_roots {
            transaction.add_scope(root);
        }
        Ok(transaction)
    }

    pub fn add_scope<P>(&mut self, root: P)
    where
        P: AsRef<Path>,
    {
        add_scope_candidates(&mut self.scope_roots, root.as_ref());
        self.scope_roots.sort();
        self.scope_roots.dedup();
    }

    /// Mark the complete controller pipeline as successful.
    ///
    /// This operation is intentionally infallible so no error can be raised
    /// after the final runtime state has already been persisted.
    pub fn commit(&mut self) {
        self.armed = false;
    }

    fn new_mount_targets(&self) -> Result<Vec<PathBuf>> {
        Ok(
            select_new_entries(read_mountinfo()?, &self.baseline_ids, &self.scope_roots)
                .into_iter()
                .map(|entry| entry.mount_point)
                .collect(),
        )
    }

    fn rollback(&mut self) -> Result<()> {
        if !self.armed {
            return Ok(());
        }

        let targets = self.new_mount_targets()?;
        let result = detach_mounts(&targets);
        if result.is_ok() {
            self.armed = false;
        }
        result
    }
}

impl Drop for MountTransaction {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }

        match self.rollback() {
            Ok(()) => {
                crate::scoped_log!(warn, "mount:rollback", "boot mount transaction rolled back")
            }
            Err(error) => crate::scoped_log!(
                error,
                "mount:rollback",
                "boot mount rollback incomplete: error={:#}",
                error
            ),
        }
    }
}

fn add_scope_candidates(roots: &mut Vec<PathBuf>, path: &Path) {
    roots.push(crate::utils::normalize_path(path));
    if let Ok(canonical) = fs::canonicalize(path) {
        roots.push(crate::utils::normalize_path(&canonical));
    }
}

fn select_new_entries(
    entries: Vec<MountEntry>,
    baseline_ids: &HashSet<u64>,
    scope_roots: &[PathBuf],
) -> Vec<MountEntry> {
    let mut selected = entries
        .into_iter()
        .filter(|entry| !baseline_ids.contains(&entry.id))
        .filter(|entry| {
            scope_roots
                .iter()
                .any(|root| entry.mount_point == *root || entry.mount_point.starts_with(root))
        })
        .collect::<Vec<_>>();

    // Mount IDs increase as mounts are created. Detaching the newest mounts
    // first also handles stacked mounts and moved Magic-Mount subtrees.
    selected.sort_by(|left, right| {
        right.id.cmp(&left.id).then_with(|| {
            right
                .mount_point
                .components()
                .count()
                .cmp(&left.mount_point.components().count())
        })
    });
    selected
}

fn detach_mounts(targets: &[PathBuf]) -> Result<()> {
    let mut errors = Vec::new();
    for target in targets {
        if let Err(error) = detach_mount(target) {
            errors.push(format!("{}: {error:#}", target.display()));
        }
    }

    if !errors.is_empty() {
        bail!("failed to detach rollback mounts: {}", errors.join(" | "));
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn detach_mount(target: &Path) -> Result<()> {
    match unmount(target, UnmountFlags::DETACH) {
        Ok(()) => Ok(()),
        Err(Errno::INVAL | Errno::NOENT) => Ok(()),
        Err(error) => {
            Err(error).with_context(|| format!("failed to detach mount {}", target.display()))
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn detach_mount(_target: &Path) -> Result<()> {
    Ok(())
}

fn read_mountinfo() -> Result<Vec<MountEntry>> {
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        Ok(Vec::new())
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        let content = fs::read_to_string("/proc/self/mountinfo")
            .context("failed to read /proc/self/mountinfo")?;
        parse_mountinfo(&content)
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn parse_mountinfo(content: &str) -> Result<Vec<MountEntry>> {
    content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            if fields.len() < 6 {
                bail!("mountinfo line has too few fields: {line}");
            }
            Ok(MountEntry {
                id: fields[0]
                    .parse::<u64>()
                    .with_context(|| format!("invalid mount id in mountinfo line: {line}"))?,
                mount_point: decode_mountinfo_path(fields[4])?,
            })
        })
        .collect()
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn decode_mountinfo_path(value: &str) -> Result<PathBuf> {
    let input = value.as_bytes();
    let mut output = Vec::with_capacity(input.len());
    let mut index = 0;

    while index < input.len() {
        if input[index] == b'\\' && index + 3 < input.len() {
            let digits = &input[index + 1..index + 4];
            if digits.iter().all(|byte| matches!(byte, b'0'..=b'7')) {
                let decoded = (digits[0] - b'0') * 64 + (digits[1] - b'0') * 8 + (digits[2] - b'0');
                output.push(decoded);
                index += 4;
                continue;
            }
        }
        output.push(input[index]);
        index += 1;
    }

    Ok(PathBuf::from(OsString::from_vec(output)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mountinfo_parser_decodes_escaped_paths() {
        let entries = parse_mountinfo(
            "41 25 0:40 / /mnt/module\\040workspace rw,relatime - tmpfs tmpfs rw\n",
        )
        .unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, 41);
        assert_eq!(entries[0].mount_point, Path::new("/mnt/module workspace"));
    }

    #[test]
    fn transaction_selects_only_new_in_scope_mounts_newest_first() {
        let baseline = HashSet::from([10, 11]);
        let entries = vec![
            MountEntry {
                id: 10,
                mount_point: PathBuf::from("/system"),
            },
            MountEntry {
                id: 13,
                mount_point: PathBuf::from("/system/app/example"),
            },
            MountEntry {
                id: 12,
                mount_point: PathBuf::from("/system"),
            },
            MountEntry {
                id: 14,
                mount_point: PathBuf::from("/unrelated"),
            },
        ];

        let selected = select_new_entries(entries, &baseline, &[PathBuf::from("/system")]);

        assert_eq!(
            selected
                .iter()
                .map(|entry| (entry.id, entry.mount_point.as_path()))
                .collect::<Vec<_>>(),
            vec![
                (13, Path::new("/system/app/example")),
                (12, Path::new("/system")),
            ]
        );
    }
}
