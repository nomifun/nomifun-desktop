/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import type { CreativeProjectDetail } from '../../domain';
import {
  CREATIVE_STUDIO_ARCHIVE_IMPORT_ENDPOINT,
  CREATIVE_STUDIO_ARCHIVE_MIME,
  createCreativeStudioHttpArchivePort,
  type CreativeStudioArchiveFetch,
} from './httpArchivePort';

const projectSummary = {
  projectId: '0190f5fe-7c00-7a00-8abc-000000000801',
  title: '归档项目',
  revision: '1',
  nodeCount: 0,
  connectionCount: 0,
  createdAt: 1,
  updatedAt: 2,
};

const projectDetail = (): CreativeProjectDetail => ({
  project: projectSummary,
  document: {
    schema: 'nomifun.creative-studio/v1',
    projectId: projectSummary.projectId,
    viewport: { x: 0, y: 0, zoom: 1 },
    background: 'lines',
    nodes: [],
    connections: [],
    chatSessions: [],
    activeChatId: null,
    panels: {
      left: { open: true, width: 288, activeView: 'canvas' },
      right: { open: true, width: 340, activeView: 'assistant' },
      bottom: { open: false, height: 240, activeView: 'history' },
    },
    pendingTaskIds: [],
  },
});

const captureError = async (operation: () => Promise<unknown>): Promise<unknown> => {
  try {
    await operation();
    return null;
  } catch (error) {
    return error;
  }
};

describe('Creative Studio HTTP archive port', () => {
  test('uploads the exact archive bytes and validates the imported summary', async () => {
    const calls: Array<{ input: string; init?: RequestInit }> = [];
    const archiveFetch: CreativeStudioArchiveFetch = async (input, init) => {
      calls.push({ input: String(input), init });
      return new Response(
        JSON.stringify({ success: true, data: { project: projectSummary } }),
        { status: 201, headers: { 'Content-Type': 'application/json' } }
      );
    };
    const port = createCreativeStudioHttpArchivePort(archiveFetch, () => undefined);
    const file = new File(['real-zip'], 'project.zip', { type: 'application/zip' });

    const imported = await port.importProjectArchive(file);

    expect(imported).toEqual([projectSummary]);
    expect(calls).toHaveLength(1);
    expect(calls[0].input.endsWith(CREATIVE_STUDIO_ARCHIVE_IMPORT_ENDPOINT)).toBe(true);
    expect(calls[0].init?.method).toBe('POST');
    expect(calls[0].init?.body).toBe(file);
    expect((calls[0].init?.headers as Record<string, string>)['Content-Type']).toBe(
      CREATIVE_STUDIO_ARCHIVE_MIME
    );
  });

  test('downloads every selected project with the server filename and real blob', async () => {
    const calls: string[] = [];
    const saved: Array<{ bytes: string; fileName: string }> = [];
    const archiveFetch: CreativeStudioArchiveFetch = async (input) => {
      calls.push(String(input));
      return new Response('zip-bytes', {
        status: 200,
        headers: {
          'Content-Type': CREATIVE_STUDIO_ARCHIVE_MIME,
          'Content-Disposition': 'attachment; filename="server-project.nomifun-canvas.zip"',
        },
      });
    };
    const port = createCreativeStudioHttpArchivePort(archiveFetch, (blob, fileName) => {
      saved.push({ bytes: String(blob.size), fileName });
    });
    const second = projectDetail();
    second.project = {
      ...second.project,
      projectId: '0190f5fe-7c00-7a00-8abc-000000000802',
    };
    second.document = { ...second.document, projectId: second.project.projectId };

    await port.exportProjectArchive([projectDetail(), second]);

    expect(calls).toHaveLength(2);
    expect(calls[0].endsWith(`/${projectSummary.projectId}/archive`)).toBe(true);
    expect(calls[1].endsWith(`/${second.project.projectId}/archive`)).toBe(true);
    expect(saved).toEqual([
      { bytes: '9', fileName: 'server-project.nomifun-canvas.zip' },
      { bytes: '9', fileName: 'server-project.nomifun-canvas.zip' },
    ]);
  });

  test('does not claim archive success for an error or wrong response type', async () => {
    const errorPort = createCreativeStudioHttpArchivePort(
      async () =>
        new Response(JSON.stringify({ success: false, error: 'bad zip', code: 'BAD_REQUEST' }), {
          status: 400,
          headers: { 'Content-Type': 'application/json' },
        }),
      () => undefined
    );
    expect(
      await captureError(() => errorPort.importProjectArchive(new File(['bad'], 'bad.zip')))
    ).toMatchObject({ name: 'BackendHttpError', status: 400 });

    const wrongTypePort = createCreativeStudioHttpArchivePort(
      async () => new Response('<html/>', { status: 200, headers: { 'Content-Type': 'text/html' } }),
      () => undefined
    );
    expect(
      await captureError(() => wrongTypePort.exportProjectArchive([projectDetail()]))
    ).toMatchObject({
      name: 'BackendHttpError',
    });
  });

});
