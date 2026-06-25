/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { QR_LOGIN_RESUME_KEY, consumeQrLoginResume } from './qrLoginResume';

const originalWindow = globalThis.window;

const installSessionStorage = () => {
  const store = new Map<string, string>();
  globalThis.window = {
    sessionStorage: {
      getItem: (key: string) => store.get(key) ?? null,
      setItem: (key: string, value: string) => {
        store.set(key, value);
      },
      removeItem: (key: string) => {
        store.delete(key);
      },
    },
  } as unknown as Window & typeof globalThis;
  return store;
};

describe('consumeQrLoginResume', () => {
  test('returns and removes a fresh QR login user', () => {
    const store = installSessionStorage();
    try {
      store.set(
        QR_LOGIN_RESUME_KEY,
        JSON.stringify({
          at: 1_000,
          user: { id: 'user_1', username: 'admin' },
        })
      );

      expect(consumeQrLoginResume(2_000)).toEqual({ id: 'user_1', username: 'admin' });
      expect(store.has(QR_LOGIN_RESUME_KEY)).toBe(false);
    } finally {
      globalThis.window = originalWindow;
    }
  });

  test('ignores expired QR login resume data', () => {
    const store = installSessionStorage();
    try {
      store.set(
        QR_LOGIN_RESUME_KEY,
        JSON.stringify({
          at: 1_000,
          user: { id: 'user_1', username: 'admin' },
        })
      );

      expect(consumeQrLoginResume(40_000)).toBe(null);
      expect(store.has(QR_LOGIN_RESUME_KEY)).toBe(false);
    } finally {
      globalThis.window = originalWindow;
    }
  });
});
