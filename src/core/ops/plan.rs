// Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-only

use std::path::PathBuf;

#[derive(Debug, Default, Clone, Copy)]
pub struct PrepareMetrics {
    pub elapsed_ms: u64,
    pub directories_scanned: usize,
    pub entries_scanned: usize,
    pub copied_entries: usize,
    pub copied_bytes: u64,
}

#[derive(Debug, Clone)]
pub struct OverlayOperation {
    pub partition_name: String,
    pub target: String,
    pub lowerdirs: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
#[cfg(feature = "kasumi")]
pub struct KasumiAddRule {
    pub target: String,
    pub source: PathBuf,
    pub file_type: i32,
}

#[derive(Debug, Clone)]
#[cfg(feature = "kasumi")]
pub struct KasumiMergeRule {
    pub target: String,
    pub source: PathBuf,
}

#[derive(Debug, Default)]
pub struct MountPlan {
    pub prepare_metrics: PrepareMetrics,
    pub overlay_ops: Vec<OverlayOperation>,
    #[cfg(feature = "kasumi")]
    pub kasumi_add_rules: Vec<KasumiAddRule>,
    #[cfg(feature = "kasumi")]
    pub kasumi_merge_rules: Vec<KasumiMergeRule>,
    #[cfg(feature = "kasumi")]
    pub kasumi_hide_rules: Vec<String>,
    pub overlay_module_ids: Vec<String>,
    pub magic_module_ids: Vec<String>,
    #[cfg(feature = "kasumi")]
    pub kasumi_module_ids: Vec<String>,
}

impl MountPlan {
    pub fn kasumi_count(&self) -> usize {
        #[cfg(feature = "kasumi")]
        {
            self.kasumi_module_ids.len()
        }
        #[cfg(not(feature = "kasumi"))]
        {
            0
        }
    }

    #[cfg(feature = "kasumi")]
    pub fn degrade_kasumi_to_magic(&mut self) -> usize {
        let mut downgraded = std::mem::take(&mut self.kasumi_module_ids);
        let changed = downgraded.len();
        self.magic_module_ids.append(&mut downgraded);
        self.magic_module_ids.sort();
        self.magic_module_ids.dedup();
        self.kasumi_add_rules.clear();
        self.kasumi_merge_rules.clear();
        self.kasumi_hide_rules.clear();
        changed
    }
}

#[cfg(all(test, feature = "kasumi"))]
mod tests {
    use std::path::PathBuf;

    use super::{KasumiAddRule, KasumiMergeRule, MountPlan};

    #[test]
    fn kasumi_downgrade_preserves_module_coverage_and_clears_rules() {
        let mut plan = MountPlan {
            magic_module_ids: vec!["existing".to_string()],
            kasumi_module_ids: vec!["b".to_string(), "a".to_string(), "existing".to_string()],
            kasumi_add_rules: vec![KasumiAddRule {
                target: "/system/a".to_string(),
                source: PathBuf::from("/dev/a"),
                file_type: 1,
            }],
            kasumi_merge_rules: vec![KasumiMergeRule {
                target: "/system/b".to_string(),
                source: PathBuf::from("/dev/b"),
            }],
            kasumi_hide_rules: vec!["/system/c".to_string()],
            ..MountPlan::default()
        };

        assert_eq!(plan.degrade_kasumi_to_magic(), 3);
        assert_eq!(plan.magic_module_ids, vec!["a", "b", "existing"]);
        assert!(plan.kasumi_module_ids.is_empty());
        assert!(plan.kasumi_add_rules.is_empty());
        assert!(plan.kasumi_merge_rules.is_empty());
        assert!(plan.kasumi_hide_rules.is_empty());
    }
}
