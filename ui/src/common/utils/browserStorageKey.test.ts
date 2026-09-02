/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { conversationTarget, parseConversationId, terminalTarget } from '@/common/types/ids';
import {
  BROWSER_STORAGE_GENERATION_STORAGE_KEY,
  browserStorageGenerationKey,
  browserStorageKey,
  getBrowserStorageGeneration,
  initializeBrowserStorageGeneration,
  sessionStorageKey,
  setBrowserStorageGeneration,
  type BrowserStoragePersistence,
} from './browserStorageKey';

const CANONICAL_GENERATION = '01900000-0000-7000-8000-000000000010';
const CANONICAL_GENERATION_2 = '01900000-0000-7000-8000-000000000011';

function createStorage(initial?: string): BrowserStoragePersistence {
  const values = new Map<string, string>();
  if (initial !== undefined) values.set(BROWSER_STORAGE_GENERATION_STORAGE_KEY, initial);
  return {
    getItem(key: string): string | null {
      return values.get(key) ?? null;
    },
    removeItem(key: string): void {
      values.delete(key);
    },
    setItem(key: string, value: string): void {
      values.set(key, value);
    },
  };
}

describe('browser storage keys', () => {
  test('includes schema version and entity namespace', () => {
    setBrowserStorageGeneration('01900000-0000-7000-8000-000000000000');
    const conversationKey = sessionStorageKey(
      'workspace-panel-tab',
      conversationTarget('0190f5fe-7c00-7a00-8000-000000000001'),
    );
    const terminalKey = sessionStorageKey(
      'workspace-panel-tab',
      terminalTarget('0190f5fe-7c00-7a00-8000-000000000001'),
    );

    expect(conversationKey.includes('|v1|')).toBe(true);
    expect(conversationKey).not.toBe(terminalKey);
  });

  test('length-prefixes segments so concatenation boundaries cannot collide', () => {
    const left = browserStorageKey(
      'ab',
      'conversation',
      parseConversationId('0190f5fe-7c00-7a00-8000-000000000001'),
    );
    const right = browserStorageKey(
      'a',
      'conversation',
      parseConversationId('0190f5fe-7c00-7a00-8000-000000000002'),
    );

    expect(left).not.toBe(right);
  });

  test('rotates when the backend dataset generation changes', () => {
    setBrowserStorageGeneration('01900000-0000-7000-8000-000000000001');
    const before = sessionStorageKey(
      'draft',
      conversationTarget('0190f5fe-7c00-7a00-8000-000000000001'),
    );
    setBrowserStorageGeneration('01900000-0000-7000-8000-000000000002');
    const after = sessionStorageKey(
      'draft',
      conversationTarget('0190f5fe-7c00-7a00-8000-000000000002'),
    );

    expect(before).not.toBe(after);
  });

  test('provides generation-scoped global feature keys without reading old keys', () => {
    setBrowserStorageGeneration('01900000-0000-7000-8000-000000000003');
    const key = browserStorageGenerationKey('cron-unread');

    expect(key.includes('|v1|')).toBe(true);
    expect(key.includes('cron-unread')).toBe(true);
    expect(key).not.toBe('nomifun_cron_unread');
  });

  test('rejects malformed or non-v7 storage generations', () => {
    for (const value of [
      '',
      'uninitialized',
      '01900000-0000-4000-8000-000000000001',
      '01900000-0000-7000-8000-000000000001 ',
      '01900000-0000-7000-C000-000000000001',
    ]) {
      let error: unknown;
      try {
        setBrowserStorageGeneration(value);
      } catch (caught) {
        error = caught;
      }
      expect(error instanceof TypeError).toBe(true);
    }
  });

  test('uses the valid backend generation and persists it for the next reload', () => {
    const storage = createStorage('01900000-0000-7000-8000-000000000012');

    const generation = initializeBrowserStorageGeneration(CANONICAL_GENERATION, storage);

    expect(generation).toBe(CANONICAL_GENERATION);
    expect(getBrowserStorageGeneration()).toBe(CANONICAL_GENERATION);
    expect(storage.getItem(BROWSER_STORAGE_GENERATION_STORAGE_KEY)).toBe(CANONICAL_GENERATION);
  });

  test('reuses a valid persisted generation when the backend is temporarily uninitialized', () => {
    const storage = createStorage(CANONICAL_GENERATION_2);

    const generation = initializeBrowserStorageGeneration('uninitialized', storage);

    expect(generation).toBe(CANONICAL_GENERATION_2);
    expect(getBrowserStorageGeneration()).toBe(CANONICAL_GENERATION_2);
  });

  test('discards an invalid persisted value and mints a canonical UUIDv7', () => {
    const storage = createStorage('legacy-storage-generation');

    const generation = initializeBrowserStorageGeneration('uninitialized', storage);

    expect(/^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(generation)).toBe(true);
    expect(generation).toBe(generation.toLowerCase());
    expect(storage.getItem(BROWSER_STORAGE_GENERATION_STORAGE_KEY)).toBe(generation);
  });

  test('reuses the persisted value across a renderer reload', () => {
    const storage = createStorage();

    const first = initializeBrowserStorageGeneration(undefined, storage);
    const afterReload = initializeBrowserStorageGeneration(undefined, storage);

    expect(afterReload).toBe(first);
    expect(afterReload).toBe(getBrowserStorageGeneration());
  });

  test('does not fail startup when browser storage is unavailable', () => {
    const unavailable: BrowserStoragePersistence = {
      getItem(_key: string): string | null {
        throw new Error('storage unavailable');
      },
      removeItem(_key: string): void {
        throw new Error('storage unavailable');
      },
      setItem(_key: string, _value: string): void {
        throw new Error('storage unavailable');
      },
    };

    const generation = initializeBrowserStorageGeneration(undefined, unavailable);

    expect(/^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(generation)).toBe(true);
    expect(generation).toBe(generation.toLowerCase());
  });
});
