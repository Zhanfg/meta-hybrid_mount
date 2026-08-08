/*
 * Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 */

import { DEFAULT_CONFIG } from "../../constants";
import type { AppConfig } from "../../types";
import { appConfigSchema } from "../schemas";

function defaultKasumi(): AppConfig["kasumi"] {
  return {
    ...DEFAULT_CONFIG.kasumi,
    mount_hide: { ...DEFAULT_CONFIG.kasumi.mount_hide },
    statfs_spoof: { ...DEFAULT_CONFIG.kasumi.statfs_spoof },
    hide_uids: [...DEFAULT_CONFIG.kasumi.hide_uids],
    uname: { ...DEFAULT_CONFIG.kasumi.uname },
    kstat_rules: [...DEFAULT_CONFIG.kasumi.kstat_rules],
    maps_rules: [...DEFAULT_CONFIG.kasumi.maps_rules],
  };
}

export function normalizeConfig(value: unknown): AppConfig {
  const parsed = appConfigSchema.parse(value);
  return {
    ...parsed,
    vfs: parsed.vfs ?? { ...DEFAULT_CONFIG.vfs },
    kasumi: parsed.kasumi ?? defaultKasumi(),
  } as AppConfig;
}
