/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Pure form logic for provider connection profiles (per-role connections,
 * `/api/providers/{id}/connections`): role validation, auth-scheme
 * classification and the credentials-form → structured-JSON mapping.
 * Kept UI-free so it is unit-testable under bun.
 */

/** Mirrors backend `validate_role` (`nomifun-system/src/provider_connection.rs`). */
export const CONNECTION_ROLE_PATTERN = /^[a-z][a-z0-9_-]{0,31}$/;

/** `default` is reserved: the providers row itself is the default connection. */
export const isValidConnectionRole = (role: string): boolean =>
  CONNECTION_ROLE_PATTERN.test(role) && role !== 'default';

/** Preset auth schemes offered in the connection drawer (plus a custom input). */
export const AUTH_SCHEME_PRESETS = [
  'bearer',
  'token',
  'header_key:x-api-key',
  'header_key:x-goog-api-key',
  'header_key:xi-api-key',
  'query_key:key',
  'query_key:api_key',
  'volc_voice',
] as const;

const PARAMETERIZED_AUTH_SCHEME_EXAMPLES: Readonly<Record<string, readonly string[]>> = {
  'header_key:<name>': [
    'header_key:x-api-key',
    'header_key:x-goog-api-key',
    'header_key:xi-api-key',
  ],
  'query_key:<param>': ['query_key:key', 'query_key:api_key'],
};

/**
 * Build resilient provider-auth suggestions.
 *
 * The backend manifest is authoritative, but it is loaded asynchronously and
 * may be unavailable while editing an already persisted provider. Parameterized
 * manifest entries are templates, not valid values by themselves, so expose
 * concrete common examples and keep the exact stored/user-entered scheme at
 * the front of the list.
 */
export const buildAuthSchemeOptions = (
  manifestSchemes: readonly string[],
  currentScheme?: string
): string[] => {
  const expand = (scheme: string): readonly string[] =>
    PARAMETERIZED_AUTH_SCHEME_EXAMPLES[scheme.trim()] ?? [scheme.trim()];
  return [
    currentScheme?.trim(),
    ...manifestSchemes.flatMap(expand),
    ...AUTH_SCHEME_PRESETS,
    // Bedrock is valid only for the provider's default connection, so it is
    // deliberately a provider-form fallback rather than a named-connection preset.
    'bedrock',
  ].filter(
    (scheme, index, values): scheme is string =>
      Boolean(scheme) && values.indexOf(scheme) === index
  );
};

export type CredentialsKind = 'api_keys' | 'volc_voice' | 'custom';

/**
 * Which credentials sub-form a scheme needs:
 * - `bearer`/`token`/`header_key:*` (and `query_key:*`) take `{api_keys: [...]}`;
 * - `volc_voice` takes the three-part app/access/resource credential;
 * - anything else falls back to a raw JSON editor.
 */
export const credentialsKindForScheme = (scheme: string): CredentialsKind => {
  const s = scheme.trim();
  if (s === 'bearer' || s === 'token' || s.startsWith('header_key:') || s.startsWith('query_key:')) {
    return 'api_keys';
  }
  if (s === 'volc_voice') return 'volc_voice';
  return 'custom';
};

/** Split an api-key textarea on commas/newlines, dropping blanks. */
export const splitApiKeys = (text: string): string[] =>
  text
    .split(/[,\n]/)
    .map((k) => k.trim())
    .filter((k) => k.length > 0);

export interface ConnectionCredentialsDraft {
  /** bearer/token/header_key textarea (comma or newline separated). */
  apiKeysText: string;
  /** volc_voice triple. */
  appKey: string;
  accessKey: string;
  resourceId: string;
  /** custom scheme raw JSON. */
  rawJson: string;
}

export type ConnectionCredentialsResult =
  /** `credentials: undefined` = nothing entered → omit on upsert (edit keeps stored). */
  | { ok: true; credentials?: Record<string, unknown> }
  | { ok: false; error: 'volc_incomplete' | 'invalid_json' | 'json_not_object' };

/**
 * Map the scheme-specific credentials form to the write-only structured JSON
 * the backend encrypts at rest. An entirely empty form yields
 * `credentials: undefined` so edit-mode saves keep the stored credentials
 * (per API contract: omitting `credentials` on upsert keeps them).
 */
export const buildConnectionCredentials = (
  scheme: string,
  draft: ConnectionCredentialsDraft
): ConnectionCredentialsResult => {
  const kind = credentialsKindForScheme(scheme);

  if (kind === 'api_keys') {
    const keys = splitApiKeys(draft.apiKeysText);
    if (keys.length === 0) return { ok: true };
    return { ok: true, credentials: { api_keys: keys } };
  }

  if (kind === 'volc_voice') {
    const appKey = draft.appKey.trim();
    const accessKey = draft.accessKey.trim();
    const resourceId = draft.resourceId.trim();
    if (!appKey && !accessKey && !resourceId) return { ok: true };
    if (!appKey || !accessKey || !resourceId) return { ok: false, error: 'volc_incomplete' };
    return { ok: true, credentials: { app_key: appKey, access_key: accessKey, resource_id: resourceId } };
  }

  const raw = draft.rawJson.trim();
  if (!raw) return { ok: true };
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return { ok: false, error: 'invalid_json' };
  }
  if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) {
    return { ok: false, error: 'json_not_object' };
  }
  return { ok: true, credentials: parsed as Record<string, unknown> };
};

/**
 * Ark / Volcengine platforms need a dedicated `role: voice` connection for
 * speech recognition / synthesis (volc voice API lives on another host with
 * its own credential scheme).
 */
export const isVolcArkPlatform = (platform: string): boolean =>
  platform === 'ark' || platform.startsWith('ark-') || platform.includes('volcengine');
