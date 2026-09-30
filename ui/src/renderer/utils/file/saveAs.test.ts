/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../test/setup-dom.ts';

import { afterEach, describe, expect, test } from 'bun:test';
import { saveBlobAs, saveUrlAs } from './saveAs';

type TestWindow = Window & {
  showSaveFilePicker?: (options: Record<string, unknown>) => Promise<unknown>;
  __backendPort?: number;
  __TAURI__?: unknown;
  __TAURI_INTERNALS__?: unknown;
};

const browserWindow = window as TestWindow;
const originalPicker = browserWindow.showSaveFilePicker;
const originalFetch = globalThis.fetch;

afterEach(() => {
  if (originalPicker) browserWindow.showSaveFilePicker = originalPicker;
  else delete browserWindow.showSaveFilePicker;
  delete browserWindow.__backendPort;
  delete browserWindow.__TAURI__;
  delete browserWindow.__TAURI_INTERNALS__;
  globalThis.fetch = originalFetch;
});

describe('Save As', () => {
  test('opens the browser destination picker before resolving file bytes', async () => {
    const order: string[] = [];
    const writes: Blob[] = [];
    let options: Record<string, unknown> | null = null;
    browserWindow.showSaveFilePicker = async (value) => {
      order.push('picker');
      options = value;
      return {
        createWritable: async () => ({
          write: async (blob: Blob) => { order.push('write'); writes.push(blob); },
          close: async () => { order.push('close'); },
        }),
      };
    };

    const result = await saveBlobAs(async () => {
      order.push('source');
      return new Blob(['image'], { type: 'image/png' });
    }, { suggestedName: 'portrait.png', mimeType: 'image/png' });

    expect(result).toEqual({ status: 'saved' });
    expect(order).toEqual(['picker', 'source', 'write', 'close']);
    expect(options).toEqual({
      suggestedName: 'portrait.png',
      types: [{ description: 'image/png', accept: { 'image/png': ['.png'] } }],
    });
    expect(writes[0]?.type).toBe('image/png');
  });

  test('does not load a remote asset when Save As is cancelled', async () => {
    let fetches = 0;
    browserWindow.showSaveFilePicker = async () => {
      throw new DOMException('cancelled', 'AbortError');
    };
    globalThis.fetch = async () => {
      fetches += 1;
      return new Response('unused');
    };

    const result = await saveUrlAs('/asset.png', {
      suggestedName: 'asset.png',
      mimeType: 'image/png',
    });

    expect(result).toEqual({ status: 'cancelled' });
    expect(fetches).toBe(0);
  });
});
