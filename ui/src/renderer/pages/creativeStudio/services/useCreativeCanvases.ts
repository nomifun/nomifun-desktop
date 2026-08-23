/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useCallback } from 'react';
import useSWR, { type SWRConfiguration } from 'swr';
import type {
  CreateCreativeCanvasRequest,
  CreativeCanvasDetail,
  CreativeCanvasDocument,
  CreativeCanvasSummary,
} from '../domain';
import {
  creativeCanvasRepository,
  type CreativeCanvasRepository,
} from './canvasRepository';

export const CREATIVE_CANVASES_SWR_KEY = 'creative-studio/canvases/v1';

export const creativeCanvasDetailKey = (
  canvasId: string
): readonly [string, string] => ['creative-studio/canvas/v1', canvasId];

export const CREATIVE_CANVAS_SWR_OPTIONS: SWRConfiguration = {
  revalidateOnFocus: false,
  shouldRetryOnError: false,
};

export function sortCreativeCanvasSummaries(
  canvases: readonly CreativeCanvasSummary[]
): CreativeCanvasSummary[] {
  return [...canvases].sort(
    (left, right) =>
      right.updatedAt - left.updatedAt ||
      left.canvasId.localeCompare(right.canvasId)
  );
}

export function upsertCreativeCanvasSummary(
  canvases: readonly CreativeCanvasSummary[] | undefined,
  canvas: CreativeCanvasSummary
): CreativeCanvasSummary[] {
  return sortCreativeCanvasSummaries([
    ...(canvases ?? []).filter(
      (candidate) => candidate.canvasId !== canvas.canvasId
    ),
    canvas,
  ]);
}

export interface CreativeCanvasesState {
  canvases: CreativeCanvasSummary[];
  isLoading: boolean;
  error: Error | undefined;
  refresh(): Promise<CreativeCanvasSummary[] | undefined>;
  create(request?: CreateCreativeCanvasRequest): Promise<CreativeCanvasSummary>;
  rename(canvasId: string, title: string): Promise<CreativeCanvasSummary>;
  remove(canvasId: string): Promise<void>;
}

/** Canvas-library query and mutations backed by the canonical repository. */
export function useCreativeCanvases(
  repository: CreativeCanvasRepository = creativeCanvasRepository
): CreativeCanvasesState {
  const { data, error, isLoading, mutate } = useSWR<
    CreativeCanvasSummary[],
    Error
  >(
    CREATIVE_CANVASES_SWR_KEY,
    () => repository.list(),
    CREATIVE_CANVAS_SWR_OPTIONS
  );

  const create = useCallback(
    async (request: CreateCreativeCanvasRequest = {}) => {
      const canvas = await repository.create(request);
      await mutate(
        (current) => upsertCreativeCanvasSummary(current, canvas),
        { revalidate: false }
      );
      return canvas;
    },
    [mutate, repository]
  );

  const rename = useCallback(
    async (canvasId: string, title: string) => {
      const canvas = await repository.rename(canvasId, title);
      await mutate(
        (current) => upsertCreativeCanvasSummary(current, canvas),
        { revalidate: false }
      );
      return canvas;
    },
    [mutate, repository]
  );

  const remove = useCallback(
    async (canvasId: string) => {
      await repository.remove(canvasId);
      await mutate(
        (current) =>
          (current ?? []).filter((canvas) => canvas.canvasId !== canvasId),
        { revalidate: false }
      );
    },
    [mutate, repository]
  );

  const refresh = useCallback(() => mutate(), [mutate]);

  return {
    canvases: sortCreativeCanvasSummaries(data ?? []),
    isLoading,
    error,
    refresh,
    create,
    rename,
    remove,
  };
}

export interface CreativeCanvasState {
  detail: CreativeCanvasDetail | undefined;
  isLoading: boolean;
  error: Error | undefined;
  refresh(): Promise<CreativeCanvasDetail | undefined>;
  save(
    expectedRevision: string,
    document: CreativeCanvasDocument
  ): Promise<CreativeCanvasSummary>;
  rename(title: string): Promise<CreativeCanvasSummary>;
  remove(): Promise<void>;
}

/** Detail query with CAS save. A falsy canvas id disables network activity. */
export function useCreativeCanvas(
  canvasId: string | null | undefined,
  repository: CreativeCanvasRepository = creativeCanvasRepository
): CreativeCanvasState {
  const { data, error, isLoading, mutate } = useSWR<
    CreativeCanvasDetail,
    Error
  >(
    canvasId ? creativeCanvasDetailKey(canvasId) : null,
    () => repository.load(canvasId as string),
    CREATIVE_CANVAS_SWR_OPTIONS
  );

  const save = useCallback(
    async (expectedRevision: string, document: CreativeCanvasDocument) => {
      if (!canvasId) throw new TypeError('Creative Studio canvas id is required');
      const canvas = await repository.save(
        canvasId,
        expectedRevision,
        document
      );
      await mutate({ canvas, document }, { revalidate: false });
      return canvas;
    },
    [canvasId, mutate, repository]
  );

  const rename = useCallback(
    async (title: string) => {
      if (!canvasId) throw new TypeError('Creative Studio canvas id is required');
      const canvas = await repository.rename(canvasId, title);
      await mutate(
        (current) => (current ? { ...current, canvas } : current),
        { revalidate: false }
      );
      return canvas;
    },
    [canvasId, mutate, repository]
  );

  const remove = useCallback(async () => {
    if (!canvasId) throw new TypeError('Creative Studio canvas id is required');
    await repository.remove(canvasId);
    await mutate(undefined, { revalidate: false });
  }, [canvasId, mutate, repository]);

  const refresh = useCallback(() => mutate(), [mutate]);

  return { detail: data, isLoading, error, refresh, save, rename, remove };
}
