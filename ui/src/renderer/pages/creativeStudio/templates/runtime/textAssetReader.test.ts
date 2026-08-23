/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import { IDS } from '../domain/testFixtures';
import { createTemplateTextAssetReader } from './textAssetReader';

describe('template planner text asset reader', () => {
  test('reads only bounded text/plain responses from the canonical asset URL', async () => {
    const calls: Array<{ input: string; init?: RequestInit }> = [];
    const reader = createTemplateTextAssetReader(
      (assetId) => `/api/creative-studio/files/${assetId}`,
      async (input, init) => {
        calls.push({ input: String(input), init });
        return new Response('{"prompts":[]}', {
          status: 200,
          headers: { 'content-type': 'text/plain; charset=utf-8' },
        });
      }
    );
    expect(await reader.read(IDS.asset)).toBe('{"prompts":[]}');
    expect(calls).toEqual([{
      input: `/api/creative-studio/files/${IDS.asset}`,
      init: { method: 'GET', credentials: 'include', signal: undefined },
    }]);
  });

  test('rejects non-text and oversized planner assets', async () => {
    const wrongType = createTemplateTextAssetReader(
      () => '/asset',
      async () => new Response('{}', { headers: { 'content-type': 'application/json' } })
    );
    try {
      await wrongType.read(IDS.asset);
      throw new Error('expected content-type rejection');
    } catch (error) {
      expect(error).toMatchObject({ code: 'asset-response' });
    }

    const oversized = createTemplateTextAssetReader(
      () => '/asset',
      async () => new Response('x', {
        headers: {
          'content-type': 'text/plain',
          'content-length': String(1024 * 1024 + 1),
        },
      })
    );
    try {
      await oversized.read(IDS.asset);
      throw new Error('expected size rejection');
    } catch (error) {
      expect(error).toMatchObject({ code: 'asset-response' });
    }
  });
});
