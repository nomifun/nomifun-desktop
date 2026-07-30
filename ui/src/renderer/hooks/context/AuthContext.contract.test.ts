/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { buildLoginRequestBody, fetchCurrentUser, parseAuthUser } from './AuthContext';

const USER_ID = '0190f5fe-7c00-7a00-8000-000000000001';

const realFetch = globalThis.fetch;

function mockFetchResponse(status: number, body?: unknown): void {
  globalThis.fetch = (() =>
    Promise.resolve(
      new Response(body === undefined ? null : JSON.stringify(body), {
        status,
        headers: body === undefined ? undefined : { 'Content-Type': 'application/json' },
      })
    )) as unknown as typeof fetch;
}

describe('fetchCurrentUser session probe classification', () => {
  // Only definitive credential rejections may end a session. A 429 from the
  // deployment-shared rate-limit bucket (docker/WebUI) or a 5xx/network blip
  // must be treated as transient — the old boolean contract bounced valid
  // sessions to the login screen whenever the server was busy.

  test('200 with a user is authenticated', async () => {
    mockFetchResponse(200, { success: true, user: { user_id: USER_ID, username: 'admin' } });
    try {
      expect(await fetchCurrentUser()).toEqual({
        kind: 'user',
        user: { id: USER_ID, username: 'admin' },
      });
    } finally {
      globalThis.fetch = realFetch;
    }
  });

  test('401 and 403 are authoritative logouts', async () => {
    try {
      for (const status of [401, 403]) {
        mockFetchResponse(status, { success: false, error: 'Forbidden: authentication required' });
        expect(await fetchCurrentUser()).toEqual({ kind: 'unauthenticated' });
      }
    } finally {
      globalThis.fetch = realFetch;
    }
  });

  test('429 and 5xx are transient, never logouts', async () => {
    try {
      for (const status of [429, 500, 502, 503]) {
        mockFetchResponse(status, { success: false, error: 'Rate limited' });
        expect(await fetchCurrentUser()).toEqual({ kind: 'transient' });
      }
    } finally {
      globalThis.fetch = realFetch;
    }
  });

  test('network failure is transient', async () => {
    globalThis.fetch = (() => Promise.reject(new TypeError('Load failed'))) as unknown as typeof fetch;
    try {
      expect(await fetchCurrentUser()).toEqual({ kind: 'transient' });
    } finally {
      globalThis.fetch = realFetch;
    }
  });
});

describe('auth user wire contract', () => {
  test('maps user_id to the UI internal id', () => {
    expect(parseAuthUser({ user_id: USER_ID, username: 'admin' })).toEqual({
      id: USER_ID,
      username: 'admin',
    });
  });

  test('rejects the legacy generic id field', () => {
    expect(parseAuthUser({ id: USER_ID, username: 'admin' })).toBe(null);
  });

  test('rejects a payload containing both user_id and generic id', () => {
    expect(parseAuthUser({ user_id: USER_ID, id: USER_ID, username: 'admin' })).toBe(null);
  });
});

describe('login request wire contract', () => {
  test('sends only the fields accepted by POST /login', () => {
    const request = buildLoginRequestBody({
      username: 'admin',
      password: 'StrongP@ss1',
    });

    expect(request).toEqual({
      username: 'admin',
      password: 'StrongP@ss1',
    });
    expect(Object.keys(request).sort()).toEqual(['password', 'username']);
  });
});
