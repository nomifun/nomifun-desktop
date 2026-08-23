/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useCallback } from 'react';
import useSWR, { type SWRConfiguration } from 'swr';
import type {
  CreateCreativeProjectRequest,
  CreativeProjectDetail,
  CreativeProjectDocument,
  CreativeProjectSummary,
} from '../domain';
import {
  creativeProjectRepository,
  type CreativeProjectRepository,
} from './projectRepository';

/** @deprecated Legacy cache isolated from canonical Canvas-shaped values. */
export const CREATIVE_PROJECTS_SWR_KEY =
  'creative-studio/legacy-project-adapter/v1';

export const creativeProjectDetailKey = (projectId: string): readonly [string, string] => [
  'creative-studio/project/v1',
  projectId,
];

export const CREATIVE_PROJECT_SWR_OPTIONS: SWRConfiguration = {
  revalidateOnFocus: false,
  shouldRetryOnError: false,
};

export function sortCreativeProjectSummaries(
  projects: readonly CreativeProjectSummary[]
): CreativeProjectSummary[] {
  return [...projects].sort(
    (left, right) => right.updatedAt - left.updatedAt || left.projectId.localeCompare(right.projectId)
  );
}

export function upsertCreativeProjectSummary(
  projects: readonly CreativeProjectSummary[] | undefined,
  project: CreativeProjectSummary
): CreativeProjectSummary[] {
  return sortCreativeProjectSummaries([
    ...(projects ?? []).filter((candidate) => candidate.projectId !== project.projectId),
    project,
  ]);
}

export interface CreativeProjectsState {
  projects: CreativeProjectSummary[];
  isLoading: boolean;
  error: Error | undefined;
  refresh(): Promise<CreativeProjectSummary[] | undefined>;
  create(request?: CreateCreativeProjectRequest): Promise<CreativeProjectSummary>;
  rename(projectId: string, title: string): Promise<CreativeProjectSummary>;
  remove(projectId: string): Promise<void>;
}

/** @deprecated Legacy hook for canvas/editor modules not migrated yet. */
export function useCreativeProjects(
  repository: CreativeProjectRepository = creativeProjectRepository
): CreativeProjectsState {
  const { data, error, isLoading, mutate } = useSWR<CreativeProjectSummary[], Error>(
    CREATIVE_PROJECTS_SWR_KEY,
    () => repository.list(),
    CREATIVE_PROJECT_SWR_OPTIONS
  );

  const create = useCallback(
    async (request: CreateCreativeProjectRequest = {}) => {
      const project = await repository.create(request);
      await mutate((current) => upsertCreativeProjectSummary(current, project), { revalidate: false });
      return project;
    },
    [mutate, repository]
  );

  const rename = useCallback(
    async (projectId: string, title: string) => {
      const project = await repository.rename(projectId, title);
      await mutate((current) => upsertCreativeProjectSummary(current, project), { revalidate: false });
      return project;
    },
    [mutate, repository]
  );

  const remove = useCallback(
    async (projectId: string) => {
      await repository.remove(projectId);
      await mutate(
        (current) => (current ?? []).filter((project) => project.projectId !== projectId),
        { revalidate: false }
      );
    },
    [mutate, repository]
  );

  const refresh = useCallback(() => mutate(), [mutate]);

  return {
    projects: sortCreativeProjectSummaries(data ?? []),
    isLoading,
    error,
    refresh,
    create,
    rename,
    remove,
  };
}

export interface CreativeProjectState {
  detail: CreativeProjectDetail | undefined;
  isLoading: boolean;
  error: Error | undefined;
  refresh(): Promise<CreativeProjectDetail | undefined>;
  save(expectedRevision: string, document: CreativeProjectDocument): Promise<CreativeProjectSummary>;
  rename(title: string): Promise<CreativeProjectSummary>;
  remove(): Promise<void>;
}

/** @deprecated Legacy hook for canvas/editor modules not migrated yet. */
export function useCreativeProject(
  projectId: string | null | undefined,
  repository: CreativeProjectRepository = creativeProjectRepository
): CreativeProjectState {
  const { data, error, isLoading, mutate } = useSWR<CreativeProjectDetail, Error>(
    projectId ? creativeProjectDetailKey(projectId) : null,
    () => repository.load(projectId as string),
    CREATIVE_PROJECT_SWR_OPTIONS
  );

  const save = useCallback(
    async (expectedRevision: string, document: CreativeProjectDocument) => {
      if (!projectId) throw new TypeError('Creative Studio canvas id is required');
      const project = await repository.save(projectId, expectedRevision, document);
      await mutate({ project, document }, { revalidate: false });
      return project;
    },
    [mutate, projectId, repository]
  );

  const rename = useCallback(
    async (title: string) => {
      if (!projectId) throw new TypeError('Creative Studio canvas id is required');
      const project = await repository.rename(projectId, title);
      await mutate((current) => (current ? { ...current, project } : current), { revalidate: false });
      return project;
    },
    [mutate, projectId, repository]
  );

  const remove = useCallback(async () => {
    if (!projectId) throw new TypeError('Creative Studio canvas id is required');
    await repository.remove(projectId);
    await mutate(undefined, { revalidate: false });
  }, [mutate, projectId, repository]);

  const refresh = useCallback(() => mutate(), [mutate]);

  return { detail: data, isLoading, error, refresh, save, rename, remove };
}
