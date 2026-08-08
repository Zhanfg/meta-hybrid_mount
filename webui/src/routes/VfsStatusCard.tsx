/*
 * Copyright (C) 2026 Hybrid Mount Developers
 * SPDX-License-Identifier: Apache-2.0
 */

import { createMemo } from "solid-js";
import { sysStore } from "../lib/stores/sysStore";
import { uiStore } from "../lib/stores/uiStore";
import "./VfsStatusCard.css";

function labels() {
  const zh = uiStore.lang.startsWith("zh");
  return zh
    ? {
        title: "VFS / No-Mount 状态",
        backend: "当前后端",
        modules: "VFS 模块",
        fallback: "本次启动回退",
        inactive: "未启用",
      }
    : {
        title: "VFS / No-Mount status",
        backend: "Active backend",
        modules: "VFS modules",
        fallback: "Boot fallbacks",
        inactive: "Inactive",
      };
}

export default function VfsStatusCard() {
  const text = createMemo(labels);
  const backend = createMemo(() => sysStore.storage.vfsBackend ?? text().inactive);

  return (
    <section class="vfs-status-card" aria-label={text().title}>
      <div class="vfs-status-heading">{text().title}</div>
      <div class="vfs-status-grid">
        <div class="vfs-status-item">
          <span>{text().backend}</span>
          <strong>{backend()}</strong>
        </div>
        <div class="vfs-status-item">
          <span>{text().modules}</span>
          <strong>{sysStore.storage.modeStats.vfs}</strong>
        </div>
        <div class="vfs-status-item">
          <span>{text().fallback}</span>
          <strong>{sysStore.storage.vfsFallbackCount}</strong>
        </div>
      </div>
    </section>
  );
}
