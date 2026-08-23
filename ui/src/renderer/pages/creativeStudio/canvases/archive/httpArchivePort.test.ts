/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import { createEmptyCreativeCanvasDocument } from '../../domain';
import {
  CREATIVE_STUDIO_CANVAS_ARCHIVE_IMPORT_ENDPOINT,
  CREATIVE_STUDIO_CANVAS_ARCHIVE_MIME,
  createCreativeStudioHttpCanvasArchivePort,
} from './httpArchivePort';

const CANVAS_ID = '0190f5fe-7c00-7a00-8abc-000000000801';
const canvas = {
  canvasId: CANVAS_ID,
  title: '归档画布',
  revision: '1',
  nodeCount: 0,
  connectionCount: 0,
  createdAt: 1,
  updatedAt: 2,
};

describe('Creative Studio Canvas HTTP archive port', () => {
  test('uploads and downloads only through Canvas endpoints', async () => {
    const calls: Array<{ input: string; method?: string }> = [];
    const saved: string[] = [];
    const port = createCreativeStudioHttpCanvasArchivePort(
      async (input, init) => {
        calls.push({ input: String(input), method: init?.method });
        if (init?.method === 'POST') {
          return new Response(
            JSON.stringify({ success: true, data: { canvas } }),
            { status: 201, headers: { 'Content-Type': 'application/json' } }
          );
        }
        return new Response('zip-bytes', {
          status: 200,
          headers: {
            'Content-Type': CREATIVE_STUDIO_CANVAS_ARCHIVE_MIME,
            'Content-Disposition':
              'attachment; filename="server-canvas.nomifun-canvas.zip"',
          },
        });
      },
      (_blob, fileName) => saved.push(fileName)
    );

    expect(
      await port.importCanvasArchive(new File(['zip'], 'canvas.zip'))
    ).toEqual([canvas]);
    await port.exportCanvasArchive([
      {
        canvas,
        document: createEmptyCreativeCanvasDocument(CANVAS_ID),
      },
    ]);

    expect(CREATIVE_STUDIO_CANVAS_ARCHIVE_IMPORT_ENDPOINT).toBe(
      '/api/creative-studio/canvases/import'
    );
    expect(
      calls[0]?.input.endsWith('/api/creative-studio/canvases/import')
    ).toBe(true);
    expect(
      calls[1]?.input.endsWith(
        `/api/creative-studio/canvases/${CANVAS_ID}/archive`
      )
    ).toBe(true);
    expect(calls.some((call) => call.input.includes('/projects'))).toBe(false);
    expect(saved).toEqual(['server-canvas.nomifun-canvas.zip']);
  });
});
