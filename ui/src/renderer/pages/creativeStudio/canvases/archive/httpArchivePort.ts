/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  BackendHttpError,
  BackendRequestError,
  buildBackendAuthHeaders,
  getBaseUrl,
} from '@/common/adapter/httpBridge';
import {
  parseCreativeCanvasResponse,
  type CreativeCanvasSummary,
} from '../../domain';
import type { CreativeStudioCanvasArchivePort } from '../canvasServiceAdapter';

export const CREATIVE_STUDIO_CANVAS_ARCHIVE_MIME =
  'application/vnd.nomifun.creative-studio+zip';
export const CREATIVE_STUDIO_CANVAS_ARCHIVE_IMPORT_ENDPOINT =
  '/api/creative-studio/canvases/import';

export type CreativeStudioCanvasArchiveFetch = (
  input: RequestInfo | URL,
  init?: RequestInit
) => Promise<Response>;

export type CreativeStudioCanvasArchiveSave = (
  blob: Blob,
  fileName: string
) => void;

const archiveExportEndpoint = (canvasId: string): string =>
  `/api/creative-studio/canvases/${encodeURIComponent(canvasId)}/archive`;

const readErrorBody = async (response: Response): Promise<unknown> => {
  const text = await response.text();
  if (!text) return '';
  try {
    return JSON.parse(text) as unknown;
  } catch {
    return text;
  }
};

const isCsrfRejection = (status: number, body: unknown): boolean =>
  status === 403 &&
  !!body &&
  typeof body === 'object' &&
  'error' in body &&
  typeof (body as { error: unknown }).error === 'string' &&
  (body as { error: string }).error.includes('CSRF token validation failed');

const requestArchive = async (
  archiveFetch: CreativeStudioCanvasArchiveFetch,
  method: 'GET' | 'POST',
  path: string,
  body?: Blob
): Promise<Response> => {
  for (let attempt = 0; attempt < 2; attempt += 1) {
    let response: Response;
    try {
      response = await archiveFetch(`${getBaseUrl()}${path}`, {
        method,
        headers: {
          ...buildBackendAuthHeaders(method),
          ...(body ? { 'Content-Type': CREATIVE_STUDIO_CANVAS_ARCHIVE_MIME } : {}),
        },
        body,
        cache: method === 'GET' ? 'no-store' : undefined,
      });
    } catch (error) {
      throw new BackendRequestError(
        'network',
        `Backend ${method} ${path} failed while transferring a Creative Studio canvas archive: ${
          error instanceof Error ? error.message : String(error)
        }`
      );
    }
    if (response.ok) return response;

    const errorBody = await readErrorBody(response);
    if (
      attempt === 0 &&
      method === 'POST' &&
      isCsrfRejection(response.status, errorBody)
    ) {
      continue;
    }
    throw new BackendHttpError({
      method,
      path,
      status: response.status,
      body: errorBody,
    });
  }
  throw new BackendRequestError(
    'network',
    `Backend ${method} ${path} failed after the CSRF retry`
  );
};

const parseImportedCanvas = async (
  response: Response
): Promise<CreativeCanvasSummary> => {
  const contentType = response.headers.get('Content-Type') ?? '';
  if (!contentType.includes('application/json')) {
    throw new BackendHttpError({
      method: 'POST',
      path: CREATIVE_STUDIO_CANVAS_ARCHIVE_IMPORT_ENDPOINT,
      status: response.status,
      body: {
        code: 'NON_JSON_RESPONSE',
        error: 'Creative Studio canvas archive import returned a non-JSON response',
      },
    });
  }
  const payload = (await response.json()) as unknown;
  const data =
    payload && typeof payload === 'object' && 'data' in payload
      ? (payload as { data: unknown }).data
      : payload;
  return parseCreativeCanvasResponse(data).canvas;
};

const parseArchiveFileName = (
  response: Response,
  canvasId: string
): string => {
  const disposition = response.headers.get('Content-Disposition') ?? '';
  const match = /(?:^|;)\s*filename="?([^";]+)"?/i.exec(disposition);
  const proposed = match?.[1]?.trim();
  if (proposed && !/[\\/:*?"<>|\u0000-\u001f]/.test(proposed)) {
    return proposed.slice(0, 180);
  }
  return `creative-studio-${canvasId}.nomifun-canvas.zip`;
};

const saveArchiveBlob: CreativeStudioCanvasArchiveSave = (blob, fileName) => {
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement('a');
  anchor.href = url;
  anchor.download = fileName;
  anchor.rel = 'noopener';
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  window.setTimeout(() => URL.revokeObjectURL(url), 1_000);
};

/** Connect the Canvas library to authenticated archive transport. */
export function createCreativeStudioHttpCanvasArchivePort(
  archiveFetch: CreativeStudioCanvasArchiveFetch = globalThis.fetch.bind(
    globalThis
  ),
  save: CreativeStudioCanvasArchiveSave = saveArchiveBlob
): Required<CreativeStudioCanvasArchivePort> {
  return {
    async importCanvasArchive(file) {
      const response = await requestArchive(
        archiveFetch,
        'POST',
        CREATIVE_STUDIO_CANVAS_ARCHIVE_IMPORT_ENDPOINT,
        file
      );
      return [await parseImportedCanvas(response)];
    },

    async exportCanvasArchive(canvases) {
      for (const detail of canvases) {
        const canvasId = detail.canvas.canvasId;
        const path = archiveExportEndpoint(canvasId);
        const response = await requestArchive(archiveFetch, 'GET', path);
        const contentType = response.headers.get('Content-Type') ?? '';
        if (!contentType.includes(CREATIVE_STUDIO_CANVAS_ARCHIVE_MIME)) {
          throw new BackendHttpError({
            method: 'GET',
            path,
            status: response.status,
            body: {
              code: 'INVALID_ARCHIVE_RESPONSE',
              error:
                'Creative Studio canvas archive export returned an unexpected content type',
            },
          });
        }
        save(await response.blob(), parseArchiveFileName(response, canvasId));
      }
    },
  };
}

export const creativeStudioHttpCanvasArchivePort =
  createCreativeStudioHttpCanvasArchivePort();
