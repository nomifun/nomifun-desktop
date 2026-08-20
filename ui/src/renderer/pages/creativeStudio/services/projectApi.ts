/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { httpRequest } from '@/common/adapter/httpBridge';
import {
  CreativeStudioContractError,
  parseCreateCreativeProjectRequest,
  parseCreativeProjectDetailResponse,
  parseCreativeProjectListResponse,
  parseCreativeProjectResponse,
  parseRenameCreativeProjectRequest,
  parseSaveCreativeProjectRequest,
  type CreateCreativeProjectRequest,
  type CreativeProjectDetail,
  type CreativeProjectResponse,
  type CreativeProjectSummary,
  type RenameCreativeProjectRequest,
  type SaveCreativeProjectRequest,
} from '../domain';

export const CREATIVE_STUDIO_PROJECTS_ENDPOINT = '/api/creative-studio/projects';

export type CreativeStudioHttpRequest = (
  method: string,
  path: string,
  body?: unknown
) => Promise<unknown>;

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

const defaultRequest: CreativeStudioHttpRequest = (method, path, body) =>
  httpRequest<unknown>(method, path, body);

const projectPath = (projectId: string): string =>
  `${CREATIVE_STUDIO_PROJECTS_ENDPOINT}/${encodeURIComponent(projectId)}`;

const projectFromResponse = (value: unknown): CreativeProjectSummary =>
  parseCreativeProjectResponse(value).project;

const assertResponseProjectId = (actual: string, expected: string): void => {
  if (actual !== expected) {
    throw new CreativeStudioContractError(
      'PROJECT_MISMATCH',
      '$.project.projectId',
      JSON.stringify(expected)
    );
  }
};

/**
 * Build a validated API client over NomiFun's shared HTTP bridge. Injection is
 * intentional: repository tests can exercise every wire path without mocking
 * global fetch, auth, or CSRF behavior.
 */
export function createCreativeStudioProjectApi(
  request: CreativeStudioHttpRequest = defaultRequest
): CreativeStudioProjectApi {
  return {
    async listProjects() {
      const response = await request('GET', CREATIVE_STUDIO_PROJECTS_ENDPOINT);
      return parseCreativeProjectListResponse(response).projects;
    },

    async createProject(input = {}) {
      const body = parseCreateCreativeProjectRequest(input);
      return projectFromResponse(await request('POST', CREATIVE_STUDIO_PROJECTS_ENDPOINT, body));
    },

    async getProject(projectId) {
      const response = await request('GET', projectPath(projectId));
      const detail = parseCreativeProjectDetailResponse(response);
      // The document parser enforces document -> summary identity; this final
      // check enforces URL -> summary identity as well.
      assertResponseProjectId(detail.project.projectId, projectId);
      return detail;
    },

    async renameProject(projectId, input) {
      const body = parseRenameCreativeProjectRequest(input);
      const project = projectFromResponse(await request('PATCH', projectPath(projectId), body));
      assertResponseProjectId(project.projectId, projectId);
      return project;
    },

    async deleteProject(projectId) {
      await request('DELETE', projectPath(projectId));
    },

    async saveProject(projectId, input) {
      const body = parseSaveCreativeProjectRequest(input, projectId);
      const response = (await request('PUT', `${projectPath(projectId)}/document`, body)) as CreativeProjectResponse;
      const project = projectFromResponse(response);
      assertResponseProjectId(project.projectId, projectId);
      return project;
    },
  };
}

export const creativeStudioProjectApi = createCreativeStudioProjectApi();
