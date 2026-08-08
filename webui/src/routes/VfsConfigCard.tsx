/*
 * Copyright (C) 2026 Hybrid Mount Developers
 * SPDX-License-Identifier: Apache-2.0
 */

import { createMemo, For, Show } from "solid-js";
import { configStore } from "../lib/stores/configStore";
import { uiStore } from "../lib/stores/uiStore";
import type { VfsBackendPreference, VfsConfig } from "../lib/types";
import "./ConfigTab.css";
import "./VfsConfigCard.css";
import "@material/web/ripple/ripple.js";

const BACKENDS: VfsBackendPreference[] = [
  "auto",
  "zeromount",
  "mirage",
  "nomountfs",
];

function copy() {
  const zh = uiStore.lang.startsWith("zh");
  return zh
    ? {
        title: "VFS / No-Mount",
        desc: "在兼容内核上使用 VFS 重定向或 No-Mount 联合挂载；不兼容时按安全规则回退。",
        enabled: "启用 VFS / No-Mount",
        disabled: "保持传统挂载",
        reboot: "此项只保存配置，并在下次启动生效；不会在线重挂载系统分区。",
        branches: "Union 分支上限",
        branchesDesc: "仅 Mirage / NoMountFS 使用，包含物理系统层，硬限制 2–5。",
        labels: {
          auto: ["自动", "优先空闲 ZeroMount，其次 Mirage，最后兼容旧 NoMountFS。"],
          zeromount: ["ZeroMount", "仅使用设备已有且空闲的 /dev/zeromount；不会接管其他模块规则。"],
          mirage: ["Mirage", "使用 Mirage union filesystem；最多 5 个总分支。"],
          nomountfs: ["Legacy NoMountFS", "仅用于兼容仍提供 nomountfs 的旧内核。"],
        },
      }
    : {
        title: "VFS / No-Mount",
        desc: "Use VFS redirection or a no-mount union backend on compatible kernels, with safety-gated fallback.",
        enabled: "Enable VFS / No-Mount",
        disabled: "Keep legacy mount flow",
        reboot: "This only saves configuration and takes effect on the next boot; system partitions are never remounted live.",
        branches: "Union branch limit",
        branchesDesc: "Mirage / NoMountFS only. Includes the physical system layer and is hard-limited to 2–5.",
        labels: {
          auto: ["Auto", "Prefer a clean ZeroMount driver, then Mirage, then legacy NoMountFS."],
          zeromount: ["ZeroMount", "Uses an existing idle /dev/zeromount only and never takes over foreign rules."],
          mirage: ["Mirage", "Uses the Mirage union filesystem with at most five total branches."],
          nomountfs: ["Legacy NoMountFS", "Compatibility path for kernels that still expose nomountfs."],
        },
      };
}

export default function VfsConfigCard() {
  const text = createMemo(copy);
  const unionBackend = createMemo(() =>
    ["auto", "mirage", "nomountfs"].includes(configStore.config.vfs.backend),
  );

  async function save(next: VfsConfig) {
    const saved = await configStore.patchConfig(
      { vfs: next },
      { applyRuntime: false, showSuccess: false },
    );
    if (saved) {
      uiStore.showToast(text().reboot, "success");
    }
    return saved;
  }

  function setEnabled(enabled: boolean) {
    return save({ ...configStore.config.vfs, enabled });
  }

  function setBackend(backend: VfsBackendPreference) {
    if (backend === configStore.config.vfs.backend) return;
    return save({ ...configStore.config.vfs, backend });
  }

  function setBranches(max_branches: number) {
    if (max_branches === configStore.config.vfs.max_branches) return;
    return save({ ...configStore.config.vfs, max_branches });
  }

  return (
    <section class="config-group vfs-config-section" aria-label="VFS / No-Mount">
      <div class="config-card">
        <div class="card-header vfs-card-header">
          <div class="card-text">
            <span class="card-title">{text().title}</span>
            <span class="card-desc">{text().desc}</span>
          </div>
          <button
            type="button"
            class={`vfs-toggle ${configStore.config.vfs.enabled ? "active" : ""}`}
            disabled={configStore.saving}
            onClick={() => setEnabled(!configStore.config.vfs.enabled)}
          >
            <md-ripple></md-ripple>
            {configStore.config.vfs.enabled ? text().enabled : text().disabled}
          </button>
        </div>

        <div class="mode-selector">
          <For each={BACKENDS}>
            {(backend) => {
              const label = () => text().labels[backend];
              return (
                <button
                  type="button"
                  class={`mode-item ${configStore.config.vfs.backend === backend ? "selected" : ""}`}
                  disabled={!configStore.config.vfs.enabled || configStore.saving}
                  onClick={() => setBackend(backend)}
                >
                  <md-ripple></md-ripple>
                  <div class="mode-info">
                    <span class="mode-title">{label()[0]}</span>
                    <span class="mode-desc">{label()[1]}</span>
                  </div>
                  <div class="mode-check" aria-hidden="true">✓</div>
                </button>
              );
            }}
          </For>
        </div>

        <Show when={configStore.config.vfs.enabled && unionBackend()}>
          <div class="vfs-branch-control">
            <div class="card-text">
              <span class="mode-title">{text().branches}</span>
              <span class="mode-desc">{text().branchesDesc}</span>
            </div>
            <div class="vfs-branch-options" role="group" aria-label={text().branches}>
              <For each={[2, 3, 4, 5]}>
                {(count) => (
                  <button
                    type="button"
                    class={configStore.config.vfs.max_branches === count ? "selected" : ""}
                    disabled={configStore.saving}
                    onClick={() => setBranches(count)}
                  >
                    {count}
                  </button>
                )}
              </For>
            </div>
          </div>
        </Show>

        <div class="vfs-reboot-note">{text().reboot}</div>
      </div>
    </section>
  );
}
