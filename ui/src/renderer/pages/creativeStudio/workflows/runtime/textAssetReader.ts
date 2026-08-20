/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { creativeAssetClient } from '../../assets';
import type { WorkflowTextAssetReader } from './types';
import { WorkflowRunRuntimeError, WorkflowTextAssetHttpError } from './types';

const MAX_PLANNER_ASSET_BYTES = 1024 * 1024;

export type WorkflowAssetUrlResolver = (assetId: string) => string;
export type WorkflowAssetFetch = (
  input: RequestInfo | URL,
  init?: RequestInit
) => Promise<Response>;

export function createWorkflowTextAssetReader(
  resolveUrl: WorkflowAssetUrlResolver = (assetId) => creativeAssetClient.url(assetId),
  fetchAsset: WorkflowAssetFetch = globalThis.fetch.bind(globalThis)
): WorkflowTextAssetReader {
  return {
    async read(assetId, signal) {
      const response = await fetchAsset(resolveUrl(assetId), {
        method: 'GET',
        credentials: 'include',
        signal,
      });
      if (!response.ok) {
        throw new WorkflowTextAssetHttpError(
          response.status,
          `planner text asset request failed with HTTP ${response.status}`
        );
      }
      const contentType = response.headers.get('content-type')?.toLowerCase() ?? '';
      if (!contentType.startsWith('text/plain')) {
        throw new WorkflowRunRuntimeError(
          'asset-response',
          `planner result must be text/plain, received ${contentType || 'no content type'}`
        );
      }
      const declaredLength = response.headers.get('content-length');
      if (declaredLength !== null) {
        const bytes = Number(declaredLength);
        if (!Number.isSafeInteger(bytes) || bytes < 0 || bytes > MAX_PLANNER_ASSET_BYTES) {
          throw new WorkflowRunRuntimeError(
            'asset-response',
            'planner text asset exceeds the 1 MiB response limit'
          );
        }
      }
      const text = await response.text();
      if (new TextEncoder().encode(text).byteLength > MAX_PLANNER_ASSET_BYTES) {
        throw new WorkflowRunRuntimeError(
          'asset-response',
          'planner text asset exceeds the 1 MiB response limit'
        );
      }
      return text;
    },
  };
}

export const workflowTextAssetReader = createWorkflowTextAssetReader();
