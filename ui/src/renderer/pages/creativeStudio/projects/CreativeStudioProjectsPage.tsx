/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useMemo } from 'react';

import CreativeStudioCanvasesPage from '../canvases/CreativeStudioCanvasesPage';
import type { CreativeStudioCanvasesService } from '../canvases/types';
import { legacyProjectCopyToCanvasCopy } from './copy';
import type { CreativeStudioProjectsCopy } from './copy';
import { creativeStudioProjectsService } from './projectServiceAdapter';
import type {
  CreativeStudioProjectSummary,
  CreativeStudioProjectsService,
  CreativeStudioProjectsSnapshot,
} from './types';

export interface CreativeStudioProjectsPageProps {
  service?: CreativeStudioProjectsService;
  onOpenProject?: (project: CreativeStudioProjectSummary) => void;
  copy?: Partial<CreativeStudioProjectsCopy>;
  initialSnapshot?: CreativeStudioProjectsSnapshot;
  initialSelectedIds?: readonly string[];
  autoLoad?: boolean;
}

const toCanvasSummary = (item: CreativeStudioProjectSummary) => ({
  canvasId: item.id,
  title: item.title,
  revision: '0',
  nodeCount: item.nodeCount,
  connectionCount: item.connectionCount,
  createdAt: item.createdAt,
  updatedAt: item.updatedAt,
});

const toLegacySummary = (
  canvas: ReturnType<typeof toCanvasSummary>
): CreativeStudioProjectSummary => ({
  id: canvas.canvasId,
  title: canvas.title,
  nodeCount: canvas.nodeCount,
  connectionCount: canvas.connectionCount,
  createdAt: canvas.createdAt,
  updatedAt: canvas.updatedAt,
});

/** @deprecated Compatibility adapter over CreativeStudioCanvasesPage. */
const CreativeStudioProjectsPage: React.FC<
  CreativeStudioProjectsPageProps
> = ({
  service = creativeStudioProjectsService,
  onOpenProject,
  copy,
  initialSnapshot,
  ...props
}) => {
  const canvasService = useMemo<CreativeStudioCanvasesService>(
    () => ({
      archiveCapabilities: service.archiveCapabilities,
      listCanvases: async (signal) =>
        (await service.listProjects(signal)).map(toCanvasSummary),
      createCanvas: async (title) =>
        toCanvasSummary(await service.createProject(title)),
      importCanvasArchive: async (file) =>
        (await service.importProjectArchive(file)).map(toCanvasSummary),
      renameCanvas: async (canvasId, title) =>
        toCanvasSummary(await service.renameProject(canvasId, title)),
      deleteCanvases: (canvasIds) => service.deleteProjects(canvasIds),
      exportCanvases: (canvasIds) => service.exportProjects(canvasIds),
    }),
    [service]
  );

  return (
    <CreativeStudioCanvasesPage
      {...props}
      service={canvasService}
      copy={legacyProjectCopyToCanvasCopy(copy)}
      initialSnapshot={
        initialSnapshot
          ? {
              status: initialSnapshot.status,
              canvases: initialSnapshot.projects.map(toCanvasSummary),
              ...(initialSnapshot.error
                ? { error: initialSnapshot.error }
                : {}),
            }
          : undefined
      }
      onOpenCanvas={(canvas) => onOpenProject?.(toLegacySummary(canvas))}
    />
  );
};

export default CreativeStudioProjectsPage;
