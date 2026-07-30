/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import {
  buildConnectionCredentials,
  credentialsKindForScheme,
  isValidConnectionRole,
  isVolcArkPlatform,
  splitApiKeys,
  type ConnectionCredentialsDraft,
} from './providerConnectionForm';

const draft = (overrides: Partial<ConnectionCredentialsDraft> = {}): ConnectionCredentialsDraft => ({
  apiKeysText: '',
  appKey: '',
  accessKey: '',
  resourceId: '',
  rawJson: '',
  ...overrides,
});

describe('connection role validation', () => {
  test('accepts backend-legal roles', () => {
    expect(isValidConnectionRole('voice')).toBe(true);
    expect(isValidConnectionRole('a')).toBe(true);
    expect(isValidConnectionRole('tts-backup_2')).toBe(true);
    expect(isValidConnectionRole('a'.repeat(32))).toBe(true);
  });

  test('rejects the reserved default role and pattern violations', () => {
    expect(isValidConnectionRole('default')).toBe(false);
    expect(isValidConnectionRole('')).toBe(false);
    expect(isValidConnectionRole('Voice')).toBe(false);
    expect(isValidConnectionRole('1voice')).toBe(false);
    expect(isValidConnectionRole('-voice')).toBe(false);
    expect(isValidConnectionRole('has space')).toBe(false);
    expect(isValidConnectionRole('a'.repeat(33))).toBe(false);
  });
});

describe('credentials kind per scheme', () => {
  test('api-key style schemes share the api_keys form', () => {
    expect(credentialsKindForScheme('bearer')).toBe('api_keys');
    expect(credentialsKindForScheme('token')).toBe('api_keys');
    expect(credentialsKindForScheme('header_key:x-goog-api-key')).toBe('api_keys');
    expect(credentialsKindForScheme('header_key:xi-api-key')).toBe('api_keys');
    expect(credentialsKindForScheme('query_key:key')).toBe('api_keys');
  });

  test('volc_voice and unknown schemes get their own forms', () => {
    expect(credentialsKindForScheme('volc_voice')).toBe('volc_voice');
    expect(credentialsKindForScheme('my_custom')).toBe('custom');
  });
});

describe('credentials form → structured JSON', () => {
  test('splits api keys on commas and newlines, dropping blanks', () => {
    expect(splitApiKeys('sk-1, sk-2\nsk-3,\n , sk-4')).toEqual(['sk-1', 'sk-2', 'sk-3', 'sk-4']);
    expect(splitApiKeys('')).toEqual([]);
  });

  test('bearer keys map to {api_keys: [...]} and empty means keep', () => {
    expect(buildConnectionCredentials('bearer', draft({ apiKeysText: 'sk-a,sk-b' }))).toEqual({
      ok: true,
      credentials: { api_keys: ['sk-a', 'sk-b'] },
    });
    expect(buildConnectionCredentials('bearer', draft())).toEqual({ ok: true });
  });

  test('volc_voice requires the full triple, all-empty means keep', () => {
    expect(
      buildConnectionCredentials(
        'volc_voice',
        draft({ appKey: 'app-1', accessKey: 'ak-1', resourceId: 'volc.bigasr.auc' })
      )
    ).toEqual({
      ok: true,
      credentials: { app_key: 'app-1', access_key: 'ak-1', resource_id: 'volc.bigasr.auc' },
    });
    expect(buildConnectionCredentials('volc_voice', draft())).toEqual({ ok: true });
    expect(buildConnectionCredentials('volc_voice', draft({ appKey: 'app-1' }))).toEqual({
      ok: false,
      error: 'volc_incomplete',
    });
  });

  test('custom scheme takes a non-empty JSON object, rejecting garbage', () => {
    expect(buildConnectionCredentials('weird', draft({ rawJson: '{"secret":"s"}' }))).toEqual({
      ok: true,
      credentials: { secret: 's' },
    });
    expect(buildConnectionCredentials('weird', draft())).toEqual({ ok: true });
    expect(buildConnectionCredentials('weird', draft({ rawJson: '###' }))).toEqual({
      ok: false,
      error: 'invalid_json',
    });
    expect(buildConnectionCredentials('weird', draft({ rawJson: '[]' }))).toEqual({
      ok: false,
      error: 'json_not_object',
    });
    expect(buildConnectionCredentials('weird', draft({ rawJson: '{}' }))).toEqual({
      ok: false,
      error: 'json_not_object',
    });
  });
});

describe('volc/ark platform detection for the voice-connection hint', () => {
  test('matches the ark family and volcengine only', () => {
    expect(isVolcArkPlatform('ark')).toBe(true);
    expect(isVolcArkPlatform('ark-coding-plan')).toBe(true);
    expect(isVolcArkPlatform('ark-agent-plan')).toBe(true);
    expect(isVolcArkPlatform('volcengine')).toBe(true);
    expect(isVolcArkPlatform('openai')).toBe(false);
    expect(isVolcArkPlatform('new-api')).toBe(false);
    expect(isVolcArkPlatform('darkhorse')).toBe(false);
  });
});
