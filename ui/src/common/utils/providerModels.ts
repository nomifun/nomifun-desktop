/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/** Readers and request mappers for the one authoritative nested model shape. */

import type { IProvider, ModelTask, ModelTrait } from '@/common/config/storage';
import type {
  CapabilityHealth,
  ProviderModelCapabilityInput,
  ProviderModelCapabilityResponse,
  ProviderModelInput,
  ProviderModelResponse,
} from '@/common/types/provider/providerModel';

export const modelOf = (
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

export const modelSupportsTask = (
  model: ProviderModelResponse,
  task: ModelTask,
  requiredTraits: readonly ModelTrait[] = []
): boolean => {
  const capability = model.capabilities.find((item) => item.task === task);
  return Boolean(
    capability && requiredTraits.every((trait) => capability.traits.includes(trait))
  );
};

/** Strip response-only health/timestamps when saving a complete model. */
export const toProviderModelCapabilityInput = (
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
});

/** Convert a response row into the full-replacement save input. */
export const toProviderModelInput = (model: ProviderModelResponse): ProviderModelInput => ({
  model: model.model,
  enabled: model.enabled,
  description: model.description,
  sort_order: model.sort_order,
  capabilities: model.capabilities.map(toProviderModelCapabilityInput),
});
