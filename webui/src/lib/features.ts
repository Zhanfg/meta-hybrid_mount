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

import { createRoot, createSignal } from "solid-js";
import { ENABLE_KASUMI } from "./constants_gen";

const createFeatures = () => {
  const [kasumiEnabled, setKasumiEnabled] = createSignal(false);
  const [kasumiAvailable, setKasumiAvailable] = createSignal(false);
  const [kasumiKernelSupported, setKasumiKernelSupported] = createSignal(false);

  return {
    get kasumiBuildEnabled() {
      return ENABLE_KASUMI;
    },
    get kasumiEnabled() {
      return ENABLE_KASUMI && kasumiEnabled();
    },
    get kasumiAvailable() {
      return ENABLE_KASUMI && kasumiAvailable();
    },
    get kasumiKernelSupported() {
      return ENABLE_KASUMI && kasumiKernelSupported();
    },
    setKasumiStatus(
      enabled: boolean,
      available: boolean,
      kernelSupported: boolean,
    ) {
      if (!ENABLE_KASUMI) return;
      setKasumiEnabled(enabled);
      setKasumiAvailable(available);
      setKasumiKernelSupported(kernelSupported);
    },
  };
};

export const features = createRoot(createFeatures);
