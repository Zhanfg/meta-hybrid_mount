// Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-only

use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum DefaultMode {
    #[default]
    Overlay,
    Magic,
    Kasumi,
}

impl DefaultMode {
    pub fn as_mount_mode(&self) -> MountMode {
        match self {
            Self::Overlay => MountMode::Overlay,
            Self::Magic => MountMode::Magic,
            Self::Kasumi => MountMode::Kasumi,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum MountMode {
    #[default]
    Overlay,
    Magic,
    Kasumi,
    Ignore,
}

impl MountMode {
    pub fn as_strategy(&self) -> &'static str {
        match self {
            Self::Overlay => "overlay",
            Self::Magic => "magic",
            Self::Kasumi => "kasumi",
            Self::Ignore => "ignore",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModuleRules {
    pub default_mode: MountMode,
    pub paths: HashMap<String, MountMode>,
}

impl ModuleRules {
    pub fn get_mode(&self, relative_path: &str) -> MountMode {
        let mut candidate = Some(relative_path);
        while let Some(path) = candidate {
            if let Some(mode) = self.paths.get(path) {
                return *mode;
            }
            candidate = path.rsplit_once('/').map(|(parent, _)| parent);
        }

        self.default_mode
    }

    pub fn effective_mode(&self, relative_path: &Path) -> MountMode {
        self.get_mode(relative_path.to_string_lossy().as_ref())
    }

    pub fn has_descendant_rule(&self, relative_path: &Path) -> bool {
        let relative = relative_path.to_string_lossy();
        let prefix = format!("{relative}/");
        self.paths.keys().any(|path| path.starts_with(&prefix))
    }

    pub fn descendant_rule_prefixes(&self) -> HashSet<String> {
        let mut prefixes = HashSet::new();
        for path in self.paths.keys() {
            let mut current = path.as_str();
            while let Some((parent, _)) = current.rsplit_once('/') {
                if parent.is_empty() {
                    break;
                }
                prefixes.insert(parent.to_string());
                current = parent;
            }
        }
        prefixes
    }

    /// Build an execution-only fallback rule set for a module whose Kasumi
    /// mirror could not be prepared. Existing Overlay, Magic and Ignore
    /// decisions are preserved; only Kasumi-selected paths become Magic.
    pub fn with_kasumi_fallback_to_magic(&self) -> Self {
        let mut fallback = self.clone();
        if matches!(fallback.default_mode, MountMode::Kasumi) {
            fallback.default_mode = MountMode::Magic;
        }
        for mode in fallback.paths.values_mut() {
            if matches!(*mode, MountMode::Kasumi) {
                *mode = MountMode::Magic;
            }
        }
        fallback
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_rules(default_mode: MountMode, paths: &[(&str, MountMode)]) -> ModuleRules {
        ModuleRules {
            default_mode,
            paths: paths.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        }
    }

    #[test]
    fn exact_match_rules() {
        // Exact path match takes precedence over prefix
        let rules = make_rules(MountMode::Overlay, &[("system", MountMode::Magic)]);
        assert_eq!(rules.get_mode("system"), MountMode::Magic);

        // Duplicate keys: later entry overwrites (HashMap semantics)
        let rules = make_rules(
            MountMode::Overlay,
            &[("sys", MountMode::Magic), ("sys", MountMode::Kasumi)],
        );
        assert_eq!(rules.get_mode("sys"), MountMode::Kasumi);
    }

    #[test]
    fn prefix_match_rules() {
        // Prefix match: "system" covers "system/app"
        let rules = make_rules(MountMode::Overlay, &[("system", MountMode::Magic)]);
        assert_eq!(rules.get_mode("system/app"), MountMode::Magic);

        // "sys" is a substring, not a path-component prefix of "system"
        let rules = make_rules(MountMode::Overlay, &[("sys", MountMode::Magic)]);
        assert_eq!(rules.get_mode("system"), MountMode::Overlay);
    }

    #[test]
    fn longest_match_wins() {
        let rules = make_rules(
            MountMode::Overlay,
            &[
                ("system", MountMode::Magic),
                ("system/app", MountMode::Kasumi),
            ],
        );
        assert_eq!(rules.get_mode("system/app/foo"), MountMode::Kasumi);
        assert_eq!(rules.get_mode("system/priv-app"), MountMode::Magic);
    }

    #[test]
    fn default_mode_rules() {
        let rules = make_rules(MountMode::Ignore, &[]);
        assert_eq!(rules.get_mode("any/path"), MountMode::Ignore);

        let rules = make_rules(MountMode::Kasumi, &[]);
        assert_eq!(rules.get_mode("system"), MountMode::Kasumi);
    }

    #[test]
    fn trailing_slash_not_prefix() {
        // "system/" is not a prefix of "system" because the slash requires
        // deeper path components
        let rules = make_rules(MountMode::Overlay, &[("system/", MountMode::Magic)]);
        assert_eq!(rules.get_mode("system"), MountMode::Overlay);
    }

    #[test]
    fn descendant_rule_prefixes_include_rule_ancestors_only() {
        let rules = make_rules(
            MountMode::Overlay,
            &[
                ("system/app/private", MountMode::Magic),
                ("vendor/lib", MountMode::Kasumi),
            ],
        );
        let prefixes = rules.descendant_rule_prefixes();

        assert!(prefixes.contains("system"));
        assert!(prefixes.contains("system/app"));
        assert!(prefixes.contains("vendor"));
        assert!(!prefixes.contains("system/app/private"));
        assert!(!prefixes.contains("vendor/lib"));
    }

    #[test]
    fn kasumi_fallback_preserves_non_kasumi_decisions() {
        let rules = make_rules(
            MountMode::Kasumi,
            &[
                ("system/app", MountMode::Overlay),
                ("system/lib", MountMode::Magic),
                ("system/etc", MountMode::Ignore),
                ("vendor/lib", MountMode::Kasumi),
            ],
        );

        let fallback = rules.with_kasumi_fallback_to_magic();

        assert_eq!(fallback.default_mode, MountMode::Magic);
        assert_eq!(fallback.get_mode("system/app/example"), MountMode::Overlay);
        assert_eq!(fallback.get_mode("system/lib/example"), MountMode::Magic);
        assert_eq!(fallback.get_mode("system/etc/example"), MountMode::Ignore);
        assert_eq!(fallback.get_mode("vendor/lib/example"), MountMode::Magic);
        assert_eq!(rules.default_mode, MountMode::Kasumi);
        assert_eq!(rules.get_mode("vendor/lib/example"), MountMode::Kasumi);
    }
}
