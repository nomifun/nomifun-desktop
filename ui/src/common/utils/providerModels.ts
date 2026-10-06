/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/** Readers and request mappers for the one authoritative nested model shape. */

import type {
  IProvider,
  ModelTask,
  ModelTechnicalCapability,
  ModelTrait,
} from '@/common/config/storage';
import type {
  CapabilityHealth,
  ProviderModelCapabilityInput,
  ProviderModelCapabilityResponse,
  ProviderModelInput,
  ProviderModelResponse,
} from '@/common/types/provider/providerModel';

const modelOf = (
  provider: Pick<IProvider, 'models'> | undefined,
  model: string
): ProviderModelResponse | undefined => provider?.models.find((row) => row.model === model);

export const capabilityOf = (
  provider: Pick<IProvider, 'models'> | undefined,
  model: string,
  task: ModelTask
): ProviderModelCapabilityResponse | undefined =>
  modelOf(provider, model)?.capabilities.find((capability) => capability.task === task);

/** Health is task-scoped; callers must identify the capability they are showing. */
export const modelHealthOf = (
  provider: Pick<IProvider, 'models'> | undefined,
  model: string,
  task: ModelTask
): CapabilityHealth | undefined => capabilityOf(provider, model, task)?.health;

/** All configured rows, including disabled rows needed by management screens. */
export const modelNamesOf = (provider: Pick<IProvider, 'models'>): string[] =>
  provider.models.map((row) => row.model);

/**
 * Input/search representation supplied by the configured Chat adapter. Saved
 * catalog traits are descriptive metadata; an omitted trait never disables a
 * native model feature. Unknown model IDs may use any supported adapter, while
 * unknown adapters cannot claim to encode inputs they do not implement.
 *
 * Keep the input protocol facts aligned with
 * `nomifun-chat-model-broker::adapter::protocol_features`.
 * Provider-native search is available through the Responses search executor.
 */
export const capabilitySupportsTrait = (
  capability: ProviderModelCapabilityResponse | undefined,
  trait: ModelTrait
): boolean => {
  if (capability?.task !== 'chat') return false;
  const protocol = capability.protocol;
  switch (trait) {
    case 'vision_input':
      return [
        'openai.chat_text',
        'openai.responses',
        'anthropic.messages',
        'gemini.generate_text',
        'bedrock.anthropic_messages',
        'vertex.anthropic_messages',
      ].includes(protocol);
    case 'audio_input':
      return ['openai.chat_text', 'openai.responses', 'gemini.generate_text'].includes(protocol);
    case 'web_search':
      return protocol === 'openai.responses';
    case 'video_input':
      return false;
  }
};

export const modelSupportsTask = (
  model: ProviderModelResponse,
  task: ModelTask,
  requiredTraits: readonly ModelTrait[] = [],
  requiredTechnicalCapabilities: readonly ModelTechnicalCapability[] = []
): boolean => {
  const capability = model.capabilities.find((item) => item.task === task);
  return Boolean(
    capability &&
      requiredTraits.every((trait) => capabilitySupportsTrait(capability, trait)) &&
      requiredTechnicalCapabilities.every(
        (technical) =>
          !capability.health?.unsupported_technical_capabilities?.includes(technical)
      )
  );
};

export const capabilitySupportsTechnicalCapability = (
  capability: ProviderModelCapabilityResponse | undefined,
  technical: ModelTechnicalCapability
): boolean =>
  Boolean(
    capability &&
      !capability.health?.unsupported_technical_capabilities?.includes(technical)
  );

/** Strip response-only health/timestamps when saving a complete model. */
const toProviderModelCapabilityInput = (
  capability: ProviderModelCapabilityResponse
): ProviderModelCapabilityInput => ({
  task: capability.task,
  traits: capability.traits,
  protocol: capability.protocol,
  connection_role: capability.connection_role,
  base_url_override: capability.base_url_override,
  endpoint: capability.endpoint,
  poll_endpoint: capability.poll_endpoint,
  content_endpoint: capability.content_endpoint,
  realtime_endpoint: capability.realtime_endpoint,
  allow_cross_origin_credentials: capability.allow_cross_origin_credentials,
  provider_params: capability.provider_params,
  context_limit: capability.context_limit,
  output_limit: capability.output_limit,
  compaction_threshold_pct: capability.compaction_threshold_pct,
});

/** Convert a response row into the full-replacement save input. */
export const toProviderModelInput = (model: ProviderModelResponse): ProviderModelInput => ({
  model: model.model,
  display_name: model.display_name,
  enabled: model.enabled,
  description: model.description,
  sort_order: model.sort_order,
  capabilities: model.capabilities.map(toProviderModelCapabilityInput),
});
