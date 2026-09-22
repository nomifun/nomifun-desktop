/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { afterEach, describe, expect, test } from 'bun:test';
import { knowledge } from './ipcBridge';

const realFetch = globalThis.fetch;

afterEach(() => {
  globalThis.fetch = realFetch;
});

describe('mutable Knowledge binding wire contract', () => {
  test('rejects the retired conversation side channel before network I/O', async () => {
    let called = false;
    globalThis.fetch = (async () => {
      called = true;
      throw new Error('must not reach fetch');
    }) as typeof fetch;

    await expect(
      knowledge.getBinding.invoke({
        kind: 'conversation',
        target_id: '0190f5fe-7c00-7a00-8000-000000000201',
      } as never)
    ).rejects.toThrow('unsupported mutable Knowledge binding kind: conversation');
    expect(called).toBe(false);
  });
});
