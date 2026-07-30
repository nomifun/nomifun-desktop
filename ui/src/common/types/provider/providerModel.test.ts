/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type {
  CreateProviderModelRequest,
  ProviderModelKeyRequest,
  ProviderModelResponse,
  UpdateProviderModelRequest,
} from './providerModel';

const PROVIDER_ID = '0190f5fe-7c00-7a00-8000-000000000002';

const wire = (value: unknown) => JSON.parse(JSON.stringify(value)) as Record<string, unknown>;

/** Full row: every optional present, mirrors provider_model.rs `full_roundtrip`. */
const fullRow: ProviderModelResponse = {
  provider_id: PROVIDER_ID,
  model: 'gpt-4o',
  enabled: true,
  sort_order: 3,
  tasks: ['chat'],
  traits: ['vision_input'],
  protocol: 'openai',
  connection_role: 'primary',
  params: { temperature: 0.7 },
  context_limit: 128_000,
  description: 'general model',
  source: 'user',
  health: {
    status: 'healthy',
    last_check: 1712345678000,
    latency: 320,
  },
  health_checked_at: 1712345678000,
  created_at: 1,
  updated_at: 2,
};

/** Minimal row as serialized by the backend: absent optionals are skipped. */
const minimalRowWire: ProviderModelResponse = {
  provider_id: PROVIDER_ID,
  model: 'gpt-4o',
  enabled: true,
  sort_order: 0,
  tasks: [],
  traits: [],
  params: null,
  source: 'inferred',
  created_at: 1,
  updated_at: 2,
};

describe('provider model wire contract (provider_model.rs mirror)', () => {
  test('full row round-trips through JSON with pinned wire field names', () => {
    const roundTripped = wire(fullRow);
    expect(Object.keys(roundTripped).sort()).toEqual([
      'connection_role',
      'context_limit',
      'created_at',
      'description',
      'enabled',
      'health',
      'health_checked_at',
      'model',
      'params',
      'protocol',
      'provider_id',
      'sort_order',
      'source',
      'tasks',
      'traits',
      'updated_at',
    ]);
    expect(roundTripped['provider_id']).toBe(PROVIDER_ID);
    expect(roundTripped['tasks']).toEqual(['chat']);
    expect(roundTripped['traits']).toEqual(['vision_input']);
    expect(roundTripped['source']).toBe('user');
    expect(roundTripped['health']).toEqual({
      status: 'healthy',
      last_check: 1712345678000,
      latency: 320,
    });
    expect(roundTripped['health_checked_at']).toBe(1712345678000);
  });

  test('backend-minimal payload is assignable: skipped optionals stay absent', () => {
    // Mirrors `minimal_serialization_skips_absent_optionals`: the backend
    // omits unset nullable columns entirely instead of sending null.
    const parsed = minimalRowWire;
    expect(parsed.source).toBe('inferred');
    expect(parsed.params).toBeNull();
    for (const absent of [
      'protocol',
      'connection_role',
      'context_limit',
      'description',
      'health',
      'health_checked_at',
    ]) {
      expect(Object.prototype.hasOwnProperty.call(parsed, absent)).toBe(false);
    }
  });

  test('create request keeps only provided fields on the wire', () => {
    const minimal: CreateProviderModelRequest = {
      provider_id: PROVIDER_ID,
      model: 'gpt-4o',
    };
    // CreateProviderModelRequest is deny_unknown_fields server-side; absent
    // optionals must be dropped from the JSON body, not sent as null.
    expect(Object.keys(wire(minimal)).sort()).toEqual(['model', 'provider_id']);

    const full: CreateProviderModelRequest = {
      provider_id: PROVIDER_ID,
      model: 'gpt-4o',
      enabled: false,
      tasks: ['speech_synthesis'],
      traits: [],
      protocol: 'openai',
      connection_role: 'voice',
      params: { speed: 1.1 },
      context_limit: 4096,
      description: 'tts model',
      sort_order: 7,
    };
    expect(Object.keys(wire(full)).sort()).toEqual([
      'connection_role',
      'context_limit',
      'description',
      'enabled',
      'model',
      'params',
      'protocol',
      'provider_id',
      'sort_order',
      'tasks',
      'traits',
    ]);
  });

  test('update request tri-state: absent = keep, explicit null = clear', () => {
    const clear: UpdateProviderModelRequest = {
      provider_id: PROVIDER_ID,
      model: 'gpt-4o',
      protocol: null,
      connection_role: null,
      context_limit: null,
      description: null,
    };
    const clearWire = wire(clear);
    // Explicit nulls must survive stringification (they mean "clear")…
    expect(clearWire['protocol']).toBeNull();
    expect(clearWire['connection_role']).toBeNull();
    expect(clearWire['context_limit']).toBeNull();
    expect(clearWire['description']).toBeNull();
    // …while untouched fields stay off the wire entirely (they mean "keep").
    for (const kept of ['enabled', 'sort_order', 'tasks', 'traits', 'params']) {
      expect(Object.prototype.hasOwnProperty.call(clearWire, kept)).toBe(false);
    }

    const set: UpdateProviderModelRequest = {
      provider_id: PROVIDER_ID,
      model: 'gpt-4o',
      enabled: false,
      sort_order: 9,
      tasks: ['chat'],
      traits: ['reasoning'],
      protocol: 'openai',
      context_limit: 200_000,
    };
    const setWire = wire(set);
    expect(setWire['enabled']).toBe(false);
    expect(setWire['sort_order']).toBe(9);
    expect(setWire['tasks']).toEqual(['chat']);
    expect(setWire['traits']).toEqual(['reasoning']);
    expect(setWire['protocol']).toBe('openai');
    expect(setWire['context_limit']).toBe(200_000);
  });

  test('key request carries exactly the composite natural key', () => {
    const key: ProviderModelKeyRequest = { provider_id: PROVIDER_ID, model: 'gpt-4o' };
    expect(wire(key)).toEqual({ provider_id: PROVIDER_ID, model: 'gpt-4o' });
  });
});
