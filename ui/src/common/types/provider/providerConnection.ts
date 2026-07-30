/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Wire-contract types for `/api/providers/{id}/connections[/{role}]` —
 * non-default per-role provider connection profiles. The providers row itself
 * remains the implicit `default` connection; these DTOs cover the extra
 * `(provider_id, role)` connection rows.
 *
 * Direct mirror of the Rust types in
 * `crates/backend/nomifun-api-types/src/provider_connection.rs`. No ts-rs:
 * keep in sync with the backend by hand.
 */

/**
 * One connection profile. Credentials are write-only: responses never echo
 * them back; `has_credentials` signals presence.
 */
export interface ProviderConnectionResponse {
  connection_id: string;
  provider_id: string;
  role: string;
  label?: string;
  base_url: string;
  auth_scheme: string;
  has_credentials: boolean;
  is_full_url: boolean;
  /** Arbitrary adapter-specific config JSON; `null` when unset. */
  extra: unknown;
  created_at: number;
  updated_at: number;
}

/**
 * Body for `POST /api/providers/{id}/connections` (upsert by `role`).
 *
 * `auth_scheme` defaults to `"bearer"` server-side. `credentials` is
 * write-only structured JSON (shape depends on `auth_scheme`), encrypted at
 * rest; omitting it on update keeps the stored credentials.
 */
export interface UpsertProviderConnectionRequest {
  role: string;
  label?: string;
  base_url: string;
  auth_scheme?: string;
  credentials?: unknown;
  is_full_url?: boolean;
  extra?: unknown;
}
