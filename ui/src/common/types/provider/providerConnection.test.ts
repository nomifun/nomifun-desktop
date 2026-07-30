/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type {
  ProviderConnectionResponse,
  UpsertProviderConnectionRequest,
} from './providerConnection';

const PROVIDER_ID = '0190f5fe-7c00-7a00-8000-000000000002';
const CONNECTION_ID = '0190f5fe-7c00-7a00-8000-000000000003';

const wire = (value: unknown) => JSON.parse(JSON.stringify(value)) as Record<string, unknown>;

const response: ProviderConnectionResponse = {
  connection_id: CONNECTION_ID,
  provider_id: PROVIDER_ID,
  role: 'voice',
  label: 'Voice endpoint',
  base_url: 'https://voice.example.com/v1',
  auth_scheme: 'bearer',
  has_credentials: true,
  is_full_url: false,
  extra: { region: 'eu' },
  created_at: 1,
  updated_at: 2,
};

describe('provider connection wire contract (provider_connection.rs mirror)', () => {
  test('response round-trips through JSON with pinned wire field names', () => {
    const roundTripped = wire(response);
    expect(Object.keys(roundTripped).sort()).toEqual([
      'auth_scheme',
      'base_url',
      'connection_id',
      'created_at',
      'extra',
      'has_credentials',
      'is_full_url',
      'label',
      'provider_id',
      'role',
      'updated_at',
    ]);
    expect(roundTripped['connection_id']).toBe(CONNECTION_ID);
    expect(roundTripped['provider_id']).toBe(PROVIDER_ID);
    expect(roundTripped['role']).toBe('voice');
    expect(roundTripped['has_credentials']).toBe(true);
    expect(roundTripped['extra']).toEqual({ region: 'eu' });
  });

  test('response is credential-free: presence is signalled, secrets never echoed', () => {
    // Mirrors `response_never_serializes_credentials`: the wire shape has no
    // credential fields at all — the TS mirror must not grow one.
    const roundTripped = wire(response);
    expect(Object.prototype.hasOwnProperty.call(roundTripped, 'credentials')).toBe(false);
    expect(Object.prototype.hasOwnProperty.call(roundTripped, 'credentials_encrypted')).toBe(false);
  });

  test('minimal backend payload is assignable: label skipped, defaults filled', () => {
    // The backend skips a None label and defaults is_full_url/extra.
    const parsed: ProviderConnectionResponse = {
      connection_id: CONNECTION_ID,
      provider_id: PROVIDER_ID,
      role: 'voice',
      base_url: 'https://voice.example.com/v1',
      auth_scheme: 'bearer',
      has_credentials: false,
      is_full_url: false,
      extra: null,
      created_at: 1,
      updated_at: 2,
    };
    expect(Object.prototype.hasOwnProperty.call(parsed, 'label')).toBe(false);
    expect(parsed.extra).toBeNull();
  });

  test('upsert request keeps only provided fields on the wire', () => {
    // UpsertProviderConnectionRequest is deny_unknown_fields server-side;
    // absent optionals must be dropped from the JSON body. Omitting
    // `credentials` on update means "keep the stored credentials".
    const minimal: UpsertProviderConnectionRequest = {
      role: 'voice',
      base_url: 'https://voice.example.com/v1',
    };
    expect(Object.keys(wire(minimal)).sort()).toEqual(['base_url', 'role']);

    const full: UpsertProviderConnectionRequest = {
      role: 'voice',
      label: 'Voice endpoint',
      base_url: 'https://voice.example.com/v1',
      auth_scheme: 'api_key',
      credentials: { api_key: 'sk-live-1234' },
      is_full_url: true,
      extra: { region: 'eu' },
    };
    const fullWire = wire(full);
    expect(Object.keys(fullWire).sort()).toEqual([
      'auth_scheme',
      'base_url',
      'credentials',
      'extra',
      'is_full_url',
      'label',
      'role',
    ]);
    expect(fullWire['auth_scheme']).toBe('api_key');
    expect(fullWire['credentials']).toEqual({ api_key: 'sk-live-1234' });
    expect(fullWire['is_full_url']).toBe(true);
  });
});
