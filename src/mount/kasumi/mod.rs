// Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-only

mod cleanup;
mod common;
mod compile;
mod runtime;
mod status;

use anyhow::{Context, Result, bail};
pub use cleanup::rollback_runtime;
pub use runtime::{apply, reset_runtime};
pub use status::{
    can_operate, collect_runtime_info, hook_lines, invalidate_runtime_caches, require_live,
};

use crate::{conf::config::Config, sys::lkm};

fn rollback_failed_runtime_update(
    config: &Config,
    lkm_was_loaded: bool,
    update_error: anyhow::Error,
) -> Result<bool> {
    let lkm_status = lkm::status(&config.kasumi)
        .context("Failed to inspect Kasumi LKM after runtime update failure")?;
    let rollback_result = if !lkm_was_loaded && lkm_status.managed {
        lkm::unload(&config.kasumi)
            .context("Failed to unload Kasumi LKM loaded by the failed runtime update")
    } else {
        cleanup::rollback_runtime().context("Failed to roll back Kasumi runtime state")
    };

    match rollback_result {
        Ok(()) => Err(update_error.context("Kasumi runtime update failed and was rolled back")),
        Err(rollback_error) => bail!(
            "Kasumi runtime update and rollback both failed: update={update_error:#}; rollback={rollback_error:#}"
        ),
    }
}

/// Apply live Kasumi configuration with rollback to the previous ownership boundary.
pub fn apply_runtime_config(config: &Config) -> Result<bool> {
    if !config.kasumi.enabled {
        let lkm_status = lkm::status(&config.kasumi)
            .context("Failed to inspect Kasumi LKM while disabling runtime configuration")?;
        if lkm_status.managed {
            lkm::unload(&config.kasumi)
                .context("Failed to unload managed Kasumi LKM while disabling configuration")?;
            return Ok(true);
        }
        if !status::can_operate(config)? {
            return Ok(false);
        }
        cleanup::rollback_runtime()?;
        return Ok(true);
    }

    let lkm_was_loaded =
        lkm::is_loaded().context("Failed to inspect Kasumi LKM before update")?;
    match runtime::apply_runtime_config(config) {
        Ok(applied) => Ok(applied),
        Err(error) => rollback_failed_runtime_update(config, lkm_was_loaded, error),
    }
}
