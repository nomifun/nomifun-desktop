/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { httpRequest } from '@/common/adapter/httpBridge';
import {
  CreativeStudioContractError,
  parseCreateCreativeCanvasRequest,
  parseCreativeCanvasDetailResponse,
  parseCreativeCanvasListResponse,
  parseCreativeCanvasResponse,
  parseRenameCreativeCanvasRequest,
  parseSaveCreativeCanvasRequest,
  type CreateCreativeCanvasRequest,
  type CreativeCanvasDetail,
  type CreativeCanvasResponse,
  type CreativeCanvasSummary,
  type RenameCreativeCanvasRequest,
  type SaveCreativeCanvasRequest,
} from '../domain';

export const CREATIVE_STUDIO_CANVASES_ENDPOINT =
  '/api/creative-studio/canvases';

export type CreativeStudioHttpRequest = (
  method: string,
  path: string,
  body?: unknown
) => Promise<unknown>;

export interface CreativeStudioCanvasApi {
  listCanvases(): Promise<CreativeCanvasSummary[]>;
  createCanvas(request?: CreateCreativeCanvasRequest): Promise<CreativeCanvasSummary>;
  getCanvas(canvasId: string): Promise<CreativeCanvasDetail>;
  renameCanvas(
    canvasId: string,
    request: RenameCreativeCanvasRequest
  ): Promise<CreativeCanvasSummary>;
  deleteCanvas(canvasId: string): Promise<void>;
  saveCanvas(
    canvasId: string,
    request: SaveCreativeCanvasRequest
  ): Promise<CreativeCanvasSummary>;
}

const defaultRequest: CreativeStudioHttpRequest = (method, path, body) =>
  httpRequest<unknown>(method, path, body);

const canvasPath = (canvasId: string): string =>
  `${CREATIVE_STUDIO_CANVASES_ENDPOINT}/${encodeURIComponent(canvasId)}`;

const canvasFromResponse = (value: unknown): CreativeCanvasSummary =>
  parseCreativeCanvasResponse(value).canvas;

const assertResponseCanvasId = (actual: string, expected: string): void => {
  if (actual !== expected) {
    throw new CreativeStudioContractError(
      'CANVAS_MISMATCH',
      '$.canvas.canvasId',
      JSON.stringify(expected)
    );
  }
};

/** Build the validated canonical Canvas API over the shared HTTP bridge. */
export function createCreativeStudioCanvasApi(
  request: CreativeStudioHttpRequest = defaultRequest
): CreativeStudioCanvasApi {
  return {
    async listCanvases() {
      const response = await request('GET', CREATIVE_STUDIO_CANVASES_ENDPOINT);
      return parseCreativeCanvasListResponse(response).canvases;
    },

    async createCanvas(input = {}) {
      const body = parseCreateCreativeCanvasRequest(input);
      return canvasFromResponse(
        await request('POST', CREATIVE_STUDIO_CANVASES_ENDPOINT, body)
      );
    },

    async getCanvas(canvasId) {
      const response = await request('GET', canvasPath(canvasId));
      const detail = parseCreativeCanvasDetailResponse(response);
      assertResponseCanvasId(detail.canvas.canvasId, canvasId);
      return detail;
    },

    async renameCanvas(canvasId, input) {
      const body = parseRenameCreativeCanvasRequest(input);
      const canvas = canvasFromResponse(
        await request('PATCH', canvasPath(canvasId), body)
      );
      assertResponseCanvasId(canvas.canvasId, canvasId);
      return canvas;
    },

    async deleteCanvas(canvasId) {
      await request('DELETE', canvasPath(canvasId));
    },

    async saveCanvas(canvasId, input) {
      const body = parseSaveCreativeCanvasRequest(input, canvasId);
      const response = (await request(
        'PUT',
        `${canvasPath(canvasId)}/document`,
        body
      )) as CreativeCanvasResponse;
      const canvas = canvasFromResponse(response);
      assertResponseCanvasId(canvas.canvasId, canvasId);
      return canvas;
    },
  };
}

export const creativeStudioCanvasApi = createCreativeStudioCanvasApi();
