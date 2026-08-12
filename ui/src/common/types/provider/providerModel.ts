/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Wire-contract types for `/api/provider-models/*`: the row-level model
 * catalog over the authoritative `provider_models` entity (composite natural
 * key `(provider_id, model)`).
 *
 * GENERATED contract: these are re-exports of the ts-rs bindings emitted from
 * `crates/backend/nomifun-api-types/src/provider_model.rs` by
 * `crates/backend/nomifun-api-types/tests/ts_export.rs`; run
 * `cargo test -p nomifun-api-types` to regenerate. Do NOT hand-edit the
 * binding files under `@/common/protocolBindings/`.
 */

export type { CapabilityHealth } from '@/common/protocolBindings/CapabilityHealth';
export type { ProviderModelCapabilityInput } from '@/common/protocolBindings/ProviderModelCapabilityInput';
export type { ProviderModelCapabilityResponse } from '@/common/protocolBindings/ProviderModelCapabilityResponse';
export type { ProviderModelInput } from '@/common/protocolBindings/ProviderModelInput';
export type { ProviderModelResponse } from '@/common/protocolBindings/ProviderModelResponse';
export type { SaveProviderModelRequest } from '@/common/protocolBindings/SaveProviderModelRequest';
export type { ProviderModelKeyRequest } from '@/common/protocolBindings/ProviderModelKeyRequest';
