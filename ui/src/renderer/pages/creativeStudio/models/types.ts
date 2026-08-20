/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IProvider, ModelTask, ModelTrait } from '@/common/config/storage';
import type { ProviderId } from '@/common/types/ids';

/** Product-level shortcuts used by Creative Studio surfaces. */
export type CreativeModelModality = 'text' | 'image' | 'video' | 'audio';

/**
 * Select a model either by a Creative Studio modality or by one exact NomiFun
 * model task. The latter keeps specialised surfaces (image edit, ASR, etc.)
 * from broadening their eligibility to a neighbouring task.
 */
export type CreativeModelFilter =
  | {
      capability: CreativeModelModality;
      traits?: readonly ModelTrait[];
    }
  | {
      capability: 'task';
      task: ModelTask;
      traits?: readonly ModelTrait[];
    };

/** Stable value persisted by Creative Studio nodes. */
export interface CreativeModelSelectionRef {
  providerId: ProviderId;
  model: string;
}

/** Fully resolved selection emitted by the picker. */
export interface CreativeModelOption extends CreativeModelSelectionRef {
  providerName: string;
  platform: string;
  task: ModelTask;
  traits: readonly ModelTrait[];
  protocol: string;
}

export interface CreativeModelGroup {
  providerId: ProviderId;
  providerName: string;
  platform: string;
  models: readonly CreativeModelOption[];
}

export type CreativeModelCatalogLoadState = 'loading' | 'ready' | 'error';

/**
 * Controlled catalog input. The NomiFun adapter produces this shape, while
 * stories and tests can supply it without booting the desktop bridge.
 */
export interface CreativeModelCatalogSnapshot {
  status: CreativeModelCatalogLoadState;
  providers: readonly IProvider[];
  error: Error | null;
  refresh?: () => void;
}

/** Narrow source shape accepted by the NomiFun provider-query adapter. */
export interface CreativeModelCatalogSource {
  data?: readonly IProvider[];
  isLoading: boolean;
  error?: unknown;
  refresh?: () => void;
}

export type CreativeModelSelectorState =
  | 'loading'
  | 'no-provider'
  | 'no-compatible-model'
  | 'disabled'
  | 'error'
  | 'ready';

export interface CreativeModelSelectCopy {
  label: string;
  placeholder: string;
  loading: string;
  noProvider: string;
  noCompatibleModel: string;
  disabled: string;
  error: string;
  unavailable: string;
  retry: string;
  configureModels: string;
}
