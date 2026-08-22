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
import { parseCreativeProjectResponse, type CreativeProjectSummary } from '../../domain';
import type { CreativeStudioProjectArchivePort } from '../projectServiceAdapter';

export const CREATIVE_STUDIO_ARCHIVE_MIME =
  'application/vnd.nomifun.creative-studio+zip';
export const CREATIVE_STUDIO_ARCHIVE_IMPORT_ENDPOINT =
  '/api/creative-studio/projects/import';

export type CreativeStudioArchiveFetch = (
  input: RequestInfo | URL,
  init?: RequestInit
) => Promise<Response>;

export type CreativeStudioArchiveSave = (blob: Blob, fileName: string) => void;

const archiveExportEndpoint = (projectId: string): string =>
  `/api/creative-studio/projects/${encodeURIComponent(projectId)}/archive`;

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
  archiveFetch: CreativeStudioArchiveFetch,
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
          ...(body ? { 'Content-Type': CREATIVE_STUDIO_ARCHIVE_MIME } : {}),
        },
        body,
        cache: method === 'GET' ? 'no-store' : undefined,
      });
    } catch (error) {
      throw new BackendRequestError(
        'network',
        `Backend ${method} ${path} failed while transferring a Creative Studio archive: ${
          error instanceof Error ? error.message : String(error)
        }`
      );
    }
    if (response.ok) return response;

    const errorBody = await readErrorBody(response);
    if (attempt === 0 && method === 'POST' && isCsrfRejection(response.status, errorBody)) {
      continue;
    }
    throw new BackendHttpError({ method, path, status: response.status, body: errorBody });
  }
  throw new BackendRequestError(
    'network',
    `Backend ${method} ${path} failed after the CSRF retry`
  );
};

const parseImportedProject = async (response: Response): Promise<CreativeProjectSummary> => {
  const contentType = response.headers.get('Content-Type') ?? '';
  if (!contentType.includes('application/json')) {
    throw new BackendHttpError({
      method: 'POST',
      path: CREATIVE_STUDIO_ARCHIVE_IMPORT_ENDPOINT,
      status: response.status,
      body: {
        code: 'NON_JSON_RESPONSE',
        error: 'Creative Studio archive import returned a non-JSON response',
      },
    });
  }
  const payload = (await response.json()) as unknown;
  const data =
    payload && typeof payload === 'object' && 'data' in payload
      ? (payload as { data: unknown }).data
      : payload;
  return parseCreativeProjectResponse(data).project;
};

const parseArchiveFileName = (response: Response, projectId: string): string => {
  const disposition = response.headers.get('Content-Disposition') ?? '';
  const match = /(?:^|;)\s*filename="?([^";]+)"?/i.exec(disposition);
  const proposed = match?.[1]?.trim();
  if (proposed && !/[\\/:*?"<>|\u0000-\u001f]/.test(proposed)) {
    return proposed.slice(0, 180);
  }
  return `creative-studio-${projectId}.nomifun-canvas.zip`;
};

const saveArchiveBlob: CreativeStudioArchiveSave = (blob, fileName) => {
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

/**
 * Connect the project center to the authenticated Creative Studio v1 archive
 * routes. Every selected project is downloaded as its own self-contained ZIP;
 * import accepts exactly one backend-validated v1 archive and returns the real
 * newly created summary.
 */
export function createCreativeStudioHttpArchivePort(
  archiveFetch: CreativeStudioArchiveFetch = globalThis.fetch.bind(globalThis),
  save: CreativeStudioArchiveSave = saveArchiveBlob
): Required<CreativeStudioProjectArchivePort> {
  return {
    async importProjectArchive(file) {
      const response = await requestArchive(
        archiveFetch,
        'POST',
        CREATIVE_STUDIO_ARCHIVE_IMPORT_ENDPOINT,
        file
      );
      return [await parseImportedProject(response)];
    },

    async exportProjectArchive(projects) {
      for (const detail of projects) {
        const projectId = detail.project.projectId;
        const path = archiveExportEndpoint(projectId);
        const response = await requestArchive(archiveFetch, 'GET', path);
        const contentType = response.headers.get('Content-Type') ?? '';
        if (!contentType.includes(CREATIVE_STUDIO_ARCHIVE_MIME)) {
          throw new BackendHttpError({
            method: 'GET',
            path,
            status: response.status,
            body: {
              code: 'INVALID_ARCHIVE_RESPONSE',
              error: 'Creative Studio archive export returned an unexpected content type',
            },
          });
        }
        save(await response.blob(), parseArchiveFileName(response, projectId));
      }
    },
  };
}

export const creativeStudioHttpArchivePort = createCreativeStudioHttpArchivePort();
