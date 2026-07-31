/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Wire-contract types for `/api/providers/{id}/connections[/{role}]` —
 * non-default per-role provider connection profiles. The providers row itself
 * remains the implicit `default` connection; these DTOs cover the extra
 * `(provider_id, role)` connection rows. Credentials are write-only: requests
 * may carry them, responses never echo them back.
 *
 * GENERATED contract: these are re-exports of the ts-rs bindings emitted from
 * `crates/backend/nomifun-api-types/src/provider_connection.rs` by
 * `crates/backend/nomifun-api-types/tests/ts_export.rs` — run
 * `cargo test -p nomifun-api-types` to regenerate. Do NOT hand-edit the
 * binding files under `@/common/protocolBindings/`.
 */

export type { ProviderConnectionResponse } from '@/common/protocolBindings/ProviderConnectionResponse';
export type { UpsertProviderConnectionRequest } from '@/common/protocolBindings/UpsertProviderConnectionRequest';
