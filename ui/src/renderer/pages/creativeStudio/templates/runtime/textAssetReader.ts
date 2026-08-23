/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { creativeAssetClient } from '../../assets';
import type { CreativeTemplateTextAssetReader } from './types';
import { CreativeTemplateRunRuntimeError, CreativeTemplateTextAssetHttpError } from './types';

const MAX_PLANNER_ASSET_BYTES = 1024 * 1024;

export type TemplateAssetUrlResolver = (assetId: string) => string;
export type TemplateAssetFetch = (
  input: RequestInfo | URL,
  init?: RequestInit
) => Promise<Response>;

export function createTemplateTextAssetReader(
  resolveUrl: TemplateAssetUrlResolver = (assetId) => creativeAssetClient.url(assetId),
  fetchAsset: TemplateAssetFetch = globalThis.fetch.bind(globalThis)
): CreativeTemplateTextAssetReader {
  return {
    async read(assetId, signal) {
      const response = await fetchAsset(resolveUrl(assetId), {
        method: 'GET',
        credentials: 'include',
        signal,
      });
      if (!response.ok) {
        throw new CreativeTemplateTextAssetHttpError(
          response.status,
          `planner text asset request failed with HTTP ${response.status}`
        );
      }
      const contentType = response.headers.get('content-type')?.toLowerCase() ?? '';
      if (!contentType.startsWith('text/plain')) {
        throw new CreativeTemplateRunRuntimeError(
          'asset-response',
          `planner result must be text/plain, received ${contentType || 'no content type'}`
        );
      }
      const declaredLength = response.headers.get('content-length');
      if (declaredLength !== null) {
        const bytes = Number(declaredLength);
        if (!Number.isSafeInteger(bytes) || bytes < 0 || bytes > MAX_PLANNER_ASSET_BYTES) {
          throw new CreativeTemplateRunRuntimeError(
            'asset-response',
            'planner text asset exceeds the 1 MiB response limit'
          );
        }
      }
      const text = await response.text();
      if (new TextEncoder().encode(text).byteLength > MAX_PLANNER_ASSET_BYTES) {
        throw new CreativeTemplateRunRuntimeError(
          'asset-response',
          'planner text asset exceeds the 1 MiB response limit'
        );
      }
      return text;
    },
  };
}

export const templateTextAssetReader = createTemplateTextAssetReader();
