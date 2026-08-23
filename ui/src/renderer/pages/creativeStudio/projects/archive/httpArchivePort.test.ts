/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import {
  CREATIVE_STUDIO_ARCHIVE_IMPORT_ENDPOINT,
  createCreativeStudioHttpArchivePort,
} from './httpArchivePort';

const CANVAS_ID = '0190f5fe-7c00-7a00-8abc-000000000801';
const summary = {
  canvasId: CANVAS_ID,
  title: '归档画布',
  revision: '1',
  nodeCount: 0,
  connectionCount: 0,
  createdAt: 1,
  updatedAt: 2,
};

describe('legacy archive module adapter', () => {
  test('targets canonical Canvas archive endpoints', async () => {
    const calls: string[] = [];
    const port = createCreativeStudioHttpArchivePort(async (input) => {
      calls.push(String(input));
      return new Response(
        JSON.stringify({ success: true, data: { canvas: summary } }),
        { status: 201, headers: { 'Content-Type': 'application/json' } }
      );
    }, () => undefined);

    expect(
      await port.importCanvasArchive(new File(['zip'], 'canvas.zip'))
    ).toEqual([summary]);
    expect(
      calls[0]?.endsWith('/api/creative-studio/canvases/import')
    ).toBe(true);
    expect(CREATIVE_STUDIO_ARCHIVE_IMPORT_ENDPOINT).toBe(
      '/api/creative-studio/canvases/import'
    );
    expect(calls[0]?.includes('/projects')).toBe(false);
  });
});
