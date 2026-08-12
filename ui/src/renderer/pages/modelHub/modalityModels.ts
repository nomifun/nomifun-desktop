/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/** Capability-first projections over `ProviderResponse.models`. */

import type { IProvider } from '@/common/config/storage';
import type { ModelTask } from '@/common/protocolBindings/ModelTask';
import type { ModelTrait } from '@/common/protocolBindings/ModelTrait';
import type {
  ProviderModelCapabilityResponse,
  ProviderModelResponse,
} from '@/common/types/provider/providerModel';
import type { ProviderId } from '@/common/types/ids';

/** Nine endpoint tasks plus the useful trait-only vision projection. */
export type ModalityKey =
  | 'chat'
  | 'realtime'
  | 'vision'
  | 'image'
  | 'image_edit'
  | 'video'
  | 'tts'
  | 'asr'
  | 'embedding'
  | 'rerank';

export interface ModalitySpec {
  task: ModelTask;
  /** Every trait must be present on this exact task capability. */
  traits: readonly ModelTrait[];
}

export const MODALITY_SPECS: Record<ModalityKey, ModalitySpec> = {
  chat: { task: 'chat', traits: [] },
  realtime: { task: 'realtime_conversation', traits: [] },
  vision: { task: 'chat', traits: ['vision_input'] },
  image: { task: 'image_generation', traits: [] },
  image_edit: { task: 'image_edit', traits: [] },
  video: { task: 'video_generation', traits: [] },
  tts: { task: 'speech_synthesis', traits: [] },
  asr: { task: 'speech_recognition', traits: [] },
  embedding: { task: 'embedding', traits: [] },
  rerank: { task: 'rerank', traits: [] },
};

export interface ModalityModelRow {
  providerId: ProviderId;
  model: string;
  enabled: boolean;
  description: string | null;
  tasks: ModelTask[];
  traits: ModelTrait[];
  protocol: string;
  /** Full response row needed by the full-replacement save endpoint. */
  definition: ProviderModelResponse;
  /** Exact capability that placed the model in this section. */
  capability: ProviderModelCapabilityResponse;
}

export interface ModalityProviderGroup {
  providerId: ProviderId;
  providerName: string;
  platform: string;
  enabled: boolean;
  models: ModalityModelRow[];
}

export const matchingCapability = (
  model: ProviderModelResponse,
  spec: ModalitySpec
): ProviderModelCapabilityResponse | undefined =>
  model.capabilities.find(
    (capability) =>
      capability.task === spec.task &&
      spec.traits.every((trait) => capability.traits.includes(trait))
  );

export const rowMatchesModality = (
  model: ProviderModelResponse,
  spec: ModalitySpec
): boolean => matchingCapability(model, spec) !== undefined;

/**
 * Management projection. Unlike runtime selectors, it deliberately retains
 * disabled providers and disabled models so users can inspect and re-enable
 * them. Provider/model ordering is the order already supplied by the backend.
 */
export const buildModalityGroups = (
  providers: readonly IProvider[],
  spec: ModalitySpec,
  providerName: (provider: IProvider) => string = (provider) => provider.name
): ModalityProviderGroup[] =>
  providers.flatMap((provider) => {
    const models = provider.models.flatMap((model): ModalityModelRow[] => {
      const capability = matchingCapability(model, spec);
      if (!capability) return [];
      return [
        {
          providerId: provider.id,
          model: model.model,
          enabled: model.enabled,
          description: model.description ?? null,
          tasks: model.capabilities.map((item) => item.task),
          traits: [...capability.traits],
          protocol: capability.protocol,
          definition: model,
          capability,
        },
      ];
    });
    if (models.length === 0) return [];
    return [
      {
        providerId: provider.id,
        providerName: providerName(provider),
        platform: provider.platform,
        enabled: provider.enabled !== false,
        models,
      },
    ];
  });
