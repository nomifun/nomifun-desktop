/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { buildBackendAuthHeaders, getBaseUrl, httpRequest } from '@/common/adapter/httpBridge';
import type { AssetId } from '@/common/types/ids';

import type { CreativeAssetSort, CreativeAssetUploadProgress } from './types';

export interface WorkshopAssetDto {
  asset_id: string;
  kind: string;
  title: string;
  collection: string | null;
  tags: unknown;
  mime: string | null;
  width: number | null;
  height: number | null;
  bytes: number | null;
  in_library: boolean;
  text_content: string | null;
  origin: unknown;
  url: string;
  thumb_url: string | null;
  created_at: number;
  updated_at: number;
}

export interface WorkshopAssetListDto {
  items: WorkshopAssetDto[];
  total: number;
}

export interface WorkshopAssetListQuery {
  kind?: string;
  collection?: string;
  q?: string;
  in_library?: boolean;
  ungrouped?: boolean;
  tag?: string;
  sort?: CreativeAssetSort;
  page?: number;
  page_size?: number;
}

export interface WorkshopAssetUploadMetadata {
  title?: string;
  collection?: string;
  tags?: string[];
  in_library?: boolean;
}

export interface WorkshopTextAssetInput {
  kind: 'text';
  title: string;
  text_content: string;
  collection?: string;
  tags?: string[];
  in_library?: boolean;
}

export interface WorkshopAssetPatch {
  title?: string;
  /** Empty string is the backend's explicit "clear collection" sentinel. */
  collection?: string;
  tags?: string[];
  in_library?: boolean;
}

export interface WorkshopAssetApi {
  list(query?: WorkshopAssetListQuery): Promise<WorkshopAssetListDto>;
  upload(
    file: File,
    metadata?: WorkshopAssetUploadMetadata,
    signal?: AbortSignal,
    onProgress?: CreativeAssetUploadProgress
  ): Promise<WorkshopAssetDto>;
  createText(input: WorkshopTextAssetInput): Promise<WorkshopAssetDto>;
  update(assetId: AssetId, patch: WorkshopAssetPatch): Promise<WorkshopAssetDto>;
  remove(assetId: AssetId): Promise<void>;
  renameCollection(from: string, to: string): Promise<number>;
  fileUrl(assetId: AssetId, thumbnail?: boolean): string;
}

export type CreativeAssetUploadErrorCode = 'aborted' | 'too_large' | 'network' | 'invalid_response' | 'http';

export class CreativeAssetUploadError extends Error {
  readonly code: CreativeAssetUploadErrorCode;
  readonly status: number | null;

  constructor(code: CreativeAssetUploadErrorCode, message: string, status: number | null = null) {
    super(message);
    this.name = 'CreativeAssetUploadError';
    this.code = code;
    this.status = status;
  }
}

function queryString(query: WorkshopAssetListQuery): string {
  const params = new URLSearchParams();
  if (query.kind) params.set('kind', query.kind);
  if (query.collection) params.set('collection', query.collection);
  if (query.q) params.set('q', query.q);
  if (query.in_library !== undefined) params.set('in_library', query.in_library ? '1' : '0');
  if (query.ungrouped !== undefined) params.set('ungrouped', query.ungrouped ? '1' : '0');
  if (query.tag) params.set('tag', query.tag);
  if (query.sort) params.set('sort', query.sort);
  if (query.page !== undefined) params.set('page', String(query.page));
  if (query.page_size !== undefined) params.set('page_size', String(query.page_size));
  const encoded = params.toString();
  return encoded ? `?${encoded}` : '';
}

function resolveBackendUrl(path: string): string {
  if (/^(?:https?:|blob:|data:)/i.test(path)) return path;
  const base = getBaseUrl();
  if (!base) return path.startsWith('/') ? path : `/${path}`;
  return path.startsWith('/') ? `${base}${path}` : `${base}/${path}`;
}

function uploadErrorMessage(xhr: XMLHttpRequest): string {
  try {
    const body = JSON.parse(xhr.responseText) as Record<string, unknown>;
    if (typeof body.error === 'string' && body.error.trim()) return body.error;
    if (typeof body.message === 'string' && body.message.trim()) return body.message;
  } catch {
    // The status text below remains the deterministic fallback.
  }
  return xhr.statusText || `HTTP ${xhr.status}`;
}

function unwrapUploadResponse(value: unknown): WorkshopAssetDto {
  const candidate =
    value && typeof value === 'object' && 'data' in value
      ? (value as { data: unknown }).data
      : value;
  if (!candidate || typeof candidate !== 'object' || typeof (candidate as { asset_id?: unknown }).asset_id !== 'string') {
    throw new CreativeAssetUploadError(
      'invalid_response',
      'Asset upload succeeded but the backend returned an invalid asset payload'
    );
  }
  return candidate as WorkshopAssetDto;
}

function uploadWorkshopAsset(
  file: File,
  metadata: WorkshopAssetUploadMetadata = {},
  signal?: AbortSignal,
  onProgress?: CreativeAssetUploadProgress
): Promise<WorkshopAssetDto> {
  return new Promise((resolve, reject) => {
    if (signal?.aborted) {
      reject(new CreativeAssetUploadError('aborted', 'Asset upload was cancelled'));
      return;
    }

    const form = new FormData();
    form.append('file', file);
    if (metadata.title?.trim()) form.append('title', metadata.title.trim());
    if (metadata.collection?.trim()) form.append('collection', metadata.collection.trim());
    if (metadata.tags?.length) form.append('tags', JSON.stringify(metadata.tags));
    if (metadata.in_library !== undefined) form.append('in_library', metadata.in_library ? '1' : '0');

    const xhr = new XMLHttpRequest();
    xhr.open('POST', resolveBackendUrl('/api/workshop/assets/upload'));
    for (const [name, value] of Object.entries(buildBackendAuthHeaders('POST'))) {
      xhr.setRequestHeader(name, value);
    }

    let settled = false;
    const detach = (): void => signal?.removeEventListener('abort', handleSignalAbort);
    const finish = (callback: () => void): void => {
      if (settled) return;
      settled = true;
      detach();
      callback();
    };
    const handleSignalAbort = (): void => xhr.abort();

    signal?.addEventListener('abort', handleSignalAbort, { once: true });
    xhr.upload.addEventListener('progress', (event) => {
      if (!onProgress || !event.lengthComputable) return;
      onProgress(Math.max(0, Math.min(100, Math.round((event.loaded / event.total) * 100))));
    });
    xhr.addEventListener('load', () => {
      finish(() => {
        if (xhr.status === 413) {
          reject(new CreativeAssetUploadError('too_large', 'Asset exceeds the backend upload limit', 413));
          return;
        }
        if (xhr.status < 200 || xhr.status >= 300) {
          reject(
            new CreativeAssetUploadError(
              'http',
              `Asset upload failed: ${uploadErrorMessage(xhr)}`,
              xhr.status
            )
          );
          return;
        }
        try {
          resolve(unwrapUploadResponse(JSON.parse(xhr.responseText) as unknown));
        } catch (error) {
          reject(
            error instanceof CreativeAssetUploadError
              ? error
              : new CreativeAssetUploadError('invalid_response', 'Asset upload returned malformed JSON')
          );
        }
      });
    });
    xhr.addEventListener('error', () => {
      finish(() => reject(new CreativeAssetUploadError('network', 'Asset upload failed because the backend is unreachable')));
    });
    xhr.addEventListener('abort', () => {
      finish(() => reject(new CreativeAssetUploadError('aborted', 'Asset upload was cancelled')));
    });

    xhr.send(form);
  });
}

export const workshopAssetApi: WorkshopAssetApi = {
  async list(query = {}) {
    const response = await httpRequest<WorkshopAssetListDto>(
      'GET',
      `/api/workshop/assets${queryString(query)}`
    );
    return { items: response?.items ?? [], total: response?.total ?? 0 };
  },

  upload: uploadWorkshopAsset,

  createText(input) {
    return httpRequest<WorkshopAssetDto>('POST', '/api/workshop/assets', input);
  },

  update(assetId, patch) {
    return httpRequest<WorkshopAssetDto>(
      'PATCH',
      `/api/workshop/assets/${encodeURIComponent(assetId)}`,
      patch
    );
  },

  async remove(assetId) {
    await httpRequest<void>('DELETE', `/api/workshop/assets/${encodeURIComponent(assetId)}`);
  },

  async renameCollection(from, to) {
    const response = await httpRequest<{ updated: number }>('POST', '/api/workshop/collections/rename', {
      from,
      to,
    });
    return response?.updated ?? 0;
  },

  fileUrl(assetId, thumbnail = false) {
    return resolveBackendUrl(
      `/api/workshop/files/${encodeURIComponent(assetId)}${thumbnail ? '?thumb=1' : ''}`
    );
  },
};
