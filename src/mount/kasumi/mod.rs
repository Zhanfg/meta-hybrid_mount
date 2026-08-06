// Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-only

mod cleanup;
mod common;
mod compile;
mod runtime;
mod status;

use anyhow::{Context, Result, bail};

use crate::conf::config::Config;

pub use cleanup::rollback_runtime;
pub use runtime::{apply, reset_runtime};
pub use status::{
    can_operate, collect_runtime_info, hook_lines, invalidate_runtime_caches, require_live,
};

/// Apply live Kasumi configuration with complete rollback semantics.
///
/// Disabling Kasumi must remove global spoof state rather than only flipping
/// the main enabled bit. Failed updates are rolled back to the same clean
/// baseline so a partially applied WebUI/CLI request cannot persist.
pub fn apply_runtime_config(config: &Config) -> Result<bool> {
    if !config.kasumi.enabled {
        if !status::can_operate(config)? {
            return Ok(false);
        }
        cleanup::rollback_runtime()?;
        return Ok(true);
    }

    match runtime::apply_runtime_config(config) {
        Ok(applied) => Ok(applied),
        Err(error) => match cleanup::rollback_runtime() {
            Ok(()) => Err(error.context("Kasumi runtime update failed and was rolled back")),
            Err(rollback_error) => bail!(
                "Kasumi runtime update and rollback both failed: update={error:#}; rollback={rollback_error:#}"
            ),
        },
    }
}
