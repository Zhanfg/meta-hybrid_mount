/*
 * Copyright (C) 2026 YuzakiKokuban <heibanbaize@gmail.com>
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

import type { SectionShellProps } from "./types";

const EXPAND_MORE_ICON = "M7.41 8.59 12 13.17l4.59-4.58L18 10l-6 6-6-6z";

export default function SectionShell(props: SectionShellProps) {
  return (
    <section
      class={`kasumi-card kasumi-section ${props.isExpanded ? "expanded" : ""}`}
    >
      <button
        class="kasumi-section-toggle"
        type="button"
        aria-expanded={props.isExpanded ? "true" : "false"}
        aria-controls={`kasumi-section-${props.id}`}
        onClick={props.onToggle}
      >
        <div class="kasumi-card-head kasumi-section-toggle-inner">
          <div>
            <div class="kasumi-card-title">{props.title}</div>
          </div>
          <div class="kasumi-section-toggle-end">
            {props.badge && (
              <div class={`state-pill ${props.badgeActive ? "active" : ""}`}>
                {props.badge}
              </div>
            )}
            <md-icon class="kasumi-section-chevron" aria-hidden="true">
              <svg viewBox="0 0 24 24">
                <path d={EXPAND_MORE_ICON} />
              </svg>
            </md-icon>
          </div>
        </div>
      </button>
      <div
        class="kasumi-section-body-wrapper"
        id={`kasumi-section-${props.id}`}
      >
        <div class="kasumi-section-body-inner">
          <div class="kasumi-section-body">{props.children}</div>
        </div>
      </div>
    </section>
  );
}
