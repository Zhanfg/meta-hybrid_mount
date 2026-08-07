// Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-only

use std::path::Path;

use anyhow::{Context, Result};

use crate::{conf::cli::Cli, defs, sys};

mod client;
pub(crate) mod protocol;
mod server;

fn ensure_private_runtime_dirs() -> Result<()> {
    sys::trusted::ensure_private_dir(Path::new(defs::HYBRID_MOUNT_DIR), 0o700)
        .context("Failed to validate private REHYBIRD data directory for daemon")?;
    sys::trusted::ensure_private_dir(Path::new(defs::RUN_DIR), 0o700)
        .context("Failed to validate private REHYBIRD run directory for daemon")?;
    Ok(())
}

pub fn dispatch(cli: &Cli, command: protocol::DaemonCommand) -> Result<()> {
    ensure_private_runtime_dirs()?;
    client::dispatch(cli, command)
}

pub fn serve() -> Result<()> {
    ensure_private_runtime_dirs()?;
    server::serve()
}
