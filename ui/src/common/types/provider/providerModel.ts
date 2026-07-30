/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Wire-contract types for `/api/provider-models/*` — the row-level model
 * catalog over the authoritative `provider_models` entity (composite natural
 * key `(provider_id, model)`).
 *
 * Direct mirror of the Rust types in
 * `crates/backend/nomifun-api-types/src/provider_model.rs` (plus
 * `ModelHealthStatus` from `crates/backend/nomifun-api-types/src/provider.rs`).
 * No ts-rs: keep in sync with the backend by hand.
 */

import type { ModelTask, ModelTrait } from '@/common/config/storage';

/**
 * Per-model health snapshot. Mirror of `ModelHealthStatus` in
 * `crates/backend/nomifun-api-types/src/provider.rs` (keep in sync); same
 * shape as the values of `IProvider['model_health']`.
 */
export interface ModelHealthStatus {
  status: 'unknown' | 'healthy' | 'unhealthy';
  /** Timestamp (ms). Absent on the wire when never checked. */
  last_check?: number;
  /** Latency in milliseconds. */
  latency?: number;
  /** Error message from the last failed check. */
  error?: string;
}

/**
 * One authoritative per-model catalog entry, projected from a
 * `provider_models` row. Identity is `(provider_id, model)`.
 *
 * Also carried on `ProviderResponse.models_detail` (and passed through to
 * `IProvider.models_detail`).
 */
export interface ProviderModelResponse {
  provider_id: string;
  model: string;
  enabled: boolean;
  sort_order: number;
  tasks: ModelTask[];
  traits: ModelTrait[];
  protocol?: string;
  connection_role?: string;
  /** Arbitrary per-model default params JSON; `null` when unset. */
  params: unknown;
  context_limit?: number;
  description?: string;
  source: 'inferred' | 'user';
  health?: ModelHealthStatus;
  health_checked_at?: number;
  created_at: number;
  updated_at: number;
}

/**
 * Body for `POST /api/provider-models` — create one catalog row.
 *
 * `tasks` left empty/absent means "no explicit profile": the backend seeds the
 * heuristic profile with `source = inferred`; a non-empty `tasks` is an
 * explicit user profile (`source = user`).
 */
export interface CreateProviderModelRequest {
  provider_id: string;
  model: string;
  enabled?: boolean;
  tasks?: ModelTask[];
  traits?: ModelTrait[];
  protocol?: string;
  connection_role?: string;
  params?: unknown;
  context_limit?: number;
  description?: string;
  sort_order?: number;
}

/**
 * Body for `POST /api/provider-models/update` — partial update of one row.
 *
 * Nullable columns are tri-state: field absent = keep, explicit `null` =
 * clear, value = set. `JSON.stringify` drops `undefined` fields, so callers
 * must send a literal `null` (not `undefined`) to clear.
 */
export interface UpdateProviderModelRequest {
  provider_id: string;
  model: string;
  enabled?: boolean;
  sort_order?: number;
  tasks?: ModelTask[];
  traits?: ModelTrait[];
  protocol?: string | null;
  connection_role?: string | null;
  params?: unknown;
  context_limit?: number | null;
  description?: string | null;
}

/**
 * Body identifying one row by its composite natural key
 * (`POST /api/provider-models/delete`).
 */
export interface ProviderModelKeyRequest {
  provider_id: string;
  model: string;
}
