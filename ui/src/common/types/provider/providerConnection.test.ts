/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type {
  ProviderConnectionInput,
  ProviderConnectionResponse,
  SaveProviderConnectionRequest,
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
  extra: { region: 'eu' },
  created_at: 1,
  updated_at: 2,
};

describe('provider connection wire contract', () => {
  test('response is metadata-only and never echoes credentials', () => {
    const body = wire(response);
    expect(body['has_credentials']).toBe(true);
    expect(Object.prototype.hasOwnProperty.call(body, 'credentials')).toBe(false);
    expect(Object.keys(body).sort()).toEqual([
      'auth_scheme',
      'base_url',
      'connection_id',
      'created_at',
      'extra',
      'has_credentials',
      'label',
      'provider_id',
      'role',
      'updated_at',
    ]);
  });

  test('one input owns role, URL, auth and write-only credentials', () => {
    const input: ProviderConnectionInput = {
      role: 'voice',
      label: 'Voice endpoint',
      base_url: 'https://voice.example.com/v1',
      auth_scheme: 'header_key:x-api-key',
      credentials: { value: 'sk-live-1234' },
      extra: { region: 'eu' },
    };
    expect(wire(input)).toEqual({
      role: 'voice',
      label: 'Voice endpoint',
      base_url: 'https://voice.example.com/v1',
      auth_scheme: 'header_key:x-api-key',
      credentials: { value: 'sk-live-1234' },
      extra: { region: 'eu' },
    });
  });

  test('aggregate create requires credentials while update may omit them to preserve', () => {
    const aggregate: ProviderConnectionInput = {
      role: 'voice',
      base_url: 'https://voice.example.com/v1',
      auth_scheme: 'bearer',
      credentials: { api_keys: ['sk-live-1234'] },
    };
    const update: SaveProviderConnectionRequest = {
      role: 'voice',
      base_url: 'https://voice.example.com/v2',
      auth_scheme: 'bearer',
    };
    expect(wire(aggregate).credentials).toEqual({ api_keys: ['sk-live-1234'] });
    expect(Object.prototype.hasOwnProperty.call(wire(update), 'credentials')).toBe(false);
  });
});
