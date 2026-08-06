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

fn rollback_newly_loaded_lkm(
    config: &Config,
    lkm_was_loaded: bool,
    update_error: anyhow::Error,
    retry_error: anyhow::Error,
) -> Result<bool> {
    let lkm_status = lkm::status(&config.kasumi)
        .context("Failed to inspect Kasumi LKM after runtime update failure")?;
    if !lkm_was_loaded && lkm_status.managed {
        return match lkm::unload(&config.kasumi) {
            Ok(()) => bail!(
                "Kasumi runtime update failed twice; the LKM loaded by this update was unloaded: first={update_error:#}; retry={retry_error:#}"
            ),
            Err(unload_error) => bail!(
                "Kasumi runtime update failed twice and the newly loaded LKM could not be unloaded: first={update_error:#}; retry={retry_error:#}; unload={unload_error:#}"
            ),
        };
    }

    bail!(
        "Kasumi runtime update failed twice; existing runtime ownership was preserved for caller rollback: first={update_error:#}; retry={retry_error:#}"
    )
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
        Err(update_error) => {
            crate::scoped_log!(
                warn,
                "mount:kasumi",
                "live runtime update failed; retrying complete rebuild once: error={:#}",
                update_error
            );
            match runtime::apply_runtime_config(config) {
                Ok(applied) => {
                    crate::scoped_log!(
                        warn,
                        "mount:kasumi",
                        "live runtime recovered on the second complete rebuild"
                    );
                    Ok(applied)
                }
                Err(retry_error) => rollback_newly_loaded_lkm(
                    config,
                    lkm_was_loaded,
                    update_error,
                    retry_error,
                ),
            }
        }
    }
}
