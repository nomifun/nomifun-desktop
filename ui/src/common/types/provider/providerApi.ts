/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Wire-contract types for `/api/providers/*`.
 *
 * Provider models and every task-scoped invocation setting have one owner:
 * `ProviderResponse.models[].capabilities[]`. Provider-level compatibility
 * maps and parallel compatibility responses are intentionally unsupported.
 */

import type { IProvider, ModelTask, ModelTrait } from '@/common/config/storage';
import { parseProviderId, type ProviderId } from '@/common/types/ids';
import type { ProviderConnectionInput } from '@/common/types/provider/providerConnection';
import type { ProviderModelInput, ProviderModelResponse } from '@/common/types/provider/providerModel';

/** Write-only credential payload selected by the explicit auth scheme. */
export type ProviderCredentials = Record<string, unknown>;

export interface ProviderResponse {
  provider_id: string;
  platform: string;
  name: string;
  base_url: string;
  auth_scheme: string;
  has_credentials: boolean;
  /** Complete authoritative model rows, including all task capabilities. */
  models: ProviderModelResponse[];
  enabled: boolean;
  bedrock_config?: IProvider['bedrock_config'];
  sort_order: number;
  created_at: number;
  updated_at: number;
}

export interface CreateProviderRequest {
  /** Optional caller-supplied UUIDv7 business id. */
  provider_id?: ProviderId;
  platform: string;
  name: string;
  base_url: string;
  auth_scheme: string;
  credentials: ProviderCredentials;
  enabled?: boolean;
  bedrock_config?: IProvider['bedrock_config'];
  sort_order?: number;
  /** Provider creation is atomic and always includes one usable model. */
  initial_model: ProviderModelInput;
  /** Named connections required by the initial capability graph. */
  connections?: ProviderConnectionInput[];
}

/** Renderer create input; only this mapper renames `id` to `provider_id`. */
export type CreateProviderInput = Omit<CreateProviderRequest, 'provider_id'> & {
  id?: ProviderId;
};

const normalizeProviderModel = (model: ProviderModelResponse): ProviderModelResponse => ({
  ...model,
  provider_id: parseProviderId(model.provider_id),
});

/** Strictly convert the provider wire response into the renderer model. */
export function fromProviderResponse(response: ProviderResponse): IProvider {
  return {
    id: parseProviderId(response.provider_id),
    platform: response.platform,
    name: response.name,
    base_url: response.base_url,
    auth_scheme: response.auth_scheme,
    has_credentials: response.has_credentials,
    models: response.models.map(normalizeProviderModel),
    enabled: response.enabled,
    bedrock_config: response.bedrock_config,
    sort_order: response.sort_order,
  };
}

/** Convert the renderer create shape into the exact backend request shape. */
export function toCreateProviderRequest(input: CreateProviderInput): CreateProviderRequest {
  return {
    ...(input.id === undefined ? {} : { provider_id: parseProviderId(input.id) }),
    platform: input.platform,
    name: input.name,
    base_url: input.base_url,
    auth_scheme: input.auth_scheme,
    credentials: input.credentials,
    enabled: input.enabled,
    bedrock_config: input.bedrock_config,
    sort_order: input.sort_order,
    initial_model: input.initial_model,
    connections: input.connections,
  };
}

/** Partial-update shape for `PUT /api/providers/:id`. */
export interface UpdateProviderRequest {
  name?: string;
  base_url?: string;
  auth_scheme?: string;
  /** Omit to preserve the encrypted credential payload already stored. */
  credentials?: ProviderCredentials;
  enabled?: boolean;
  bedrock_config?: IProvider['bedrock_config'];
  sort_order?: number;
}

/**
 * Allow-list the exact update contract at the HTTP boundary so a renderer
 * record can never leak response-only nested model data back to this route.
 */
export function toUpdateProviderRequest(input: UpdateProviderRequest): UpdateProviderRequest {
  const { name, base_url, auth_scheme, credentials, enabled, bedrock_config, sort_order } = input;
  return {
    ...(name === undefined ? {} : { name }),
    ...(base_url === undefined ? {} : { base_url }),
    ...(auth_scheme === undefined ? {} : { auth_scheme }),
    ...(credentials === undefined ? {} : { credentials }),
    ...(enabled === undefined ? {} : { enabled }),
    ...(bedrock_config === undefined ? {} : { bedrock_config }),
    ...(sort_order === undefined ? {} : { sort_order }),
  };
}

/**
 * One fixed fetch-model shape. There is no string or profile-map fallback.
 *
 * Mirrors the ts-rs generated `ModelInfo`. `context_limit` is present only when
 * the provider's own catalog declares a window (Gemini's `inputTokenLimit`, an
 * OpenAI-compatible gateway's `context_length`); it is absent, never null, when
 * the provider says nothing.
 */
export interface FetchedModelInfo {
  id: string;
  name?: string | null;
  tasks?: ModelTask[];
  traits?: ModelTrait[];
  context_limit?: number;
}

export interface FetchModelsResponse {
  models: FetchedModelInfo[];
  /** Present when the backend identifies the official model-list origin. */
  fixed_base_url?: string;
}

/** Anonymous model discovery used before a provider exists. */
export interface FetchModelsAnonymousRequest {
  platform: string;
  base_url: string;
  auth_scheme: string;
  credentials: ProviderCredentials;
  bedrock_config?: IProvider['bedrock_config'];
  try_fix?: boolean;
}

export type { ProviderHealthCheckErrorKind } from '@/common/protocolBindings/ProviderHealthCheckErrorKind';
export type { ProviderHealthCheckRequest } from '@/common/protocolBindings/ProviderHealthCheckRequest';
export type { ProviderHealthCheckResponse } from '@/common/protocolBindings/ProviderHealthCheckResponse';
