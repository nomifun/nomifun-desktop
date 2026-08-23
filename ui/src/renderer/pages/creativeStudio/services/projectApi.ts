/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  creativeCanvasDetailToLegacyProject,
  creativeCanvasSummaryToLegacyProject,
  legacyProjectDocumentToCreativeCanvas,
  type CreateCreativeProjectRequest,
  type CreativeProjectDetail,
  type CreativeProjectSummary,
  type RenameCreativeProjectRequest,
  type SaveCreativeProjectRequest,
} from '../domain';
import {
  CREATIVE_STUDIO_CANVASES_ENDPOINT,
  createCreativeStudioCanvasApi,
  creativeStudioCanvasApi,
  type CreativeStudioCanvasApi,
  type CreativeStudioHttpRequest,
} from './canvasApi';

/**
 * @deprecated Historical export retained for canvas/editor modules that have
 * not migrated their in-process names. It intentionally targets `/canvases`.
 */
export const CREATIVE_STUDIO_PROJECTS_ENDPOINT =
  CREATIVE_STUDIO_CANVASES_ENDPOINT;

export type { CreativeStudioHttpRequest };

/** @deprecated Use CreativeStudioCanvasApi. */
export interface CreativeStudioProjectApi {
  listProjects(): Promise<CreativeProjectSummary[]>;
  createProject(request?: CreateCreativeProjectRequest): Promise<CreativeProjectSummary>;
  getProject(projectId: string): Promise<CreativeProjectDetail>;
  renameProject(
    projectId: string,
    request: RenameCreativeProjectRequest
  ): Promise<CreativeProjectSummary>;
  deleteProject(projectId: string): Promise<void>;
  saveProject(
    projectId: string,
    request: SaveCreativeProjectRequest
  ): Promise<CreativeProjectSummary>;
}

/**
 * @deprecated Legacy in-process shape adapter over the canonical Canvas API.
 * Passing an HTTP request remains supported for focused compatibility tests.
 */
export function createCreativeStudioProjectApi(
  input: CreativeStudioCanvasApi | CreativeStudioHttpRequest =
    creativeStudioCanvasApi
): CreativeStudioProjectApi {
  const api =
    typeof input === 'function' ? createCreativeStudioCanvasApi(input) : input;

  return {
    async listProjects() {
      return (await api.listCanvases()).map(creativeCanvasSummaryToLegacyProject);
    },

    async createProject(request = {}) {
      return creativeCanvasSummaryToLegacyProject(
        await api.createCanvas(request)
      );
    },

    async getProject(projectId) {
      return creativeCanvasDetailToLegacyProject(
        await api.getCanvas(projectId)
      );
    },

    async renameProject(projectId, request) {
      return creativeCanvasSummaryToLegacyProject(
        await api.renameCanvas(projectId, request)
      );
    },

    deleteProject: (projectId) => api.deleteCanvas(projectId),

    async saveProject(projectId, request) {
      return creativeCanvasSummaryToLegacyProject(
        await api.saveCanvas(projectId, {
          expectedRevision: request.expectedRevision,
          document: legacyProjectDocumentToCreativeCanvas(request.document),
        })
      );
    },
  };
}

/** @deprecated Use creativeStudioCanvasApi. */
export const creativeStudioProjectApi = createCreativeStudioProjectApi();
