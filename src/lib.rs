// Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
//
// SPDX-License-Identifier: GPL-3.0-only

// Flash-candidate revalidation marker: build and lint this exact source tree.
pub mod conf;
pub mod core;
pub mod defs;
pub mod domain;
#[cfg(feature = "kasumi")]
pub mod kasumi_runtime_safety;
pub mod mount;
pub mod partitions;
pub mod path_safety;
pub mod sys;
pub mod utils;
