/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';

import CreativeStudioCanvasCard from '../canvases/CreativeStudioCanvasCard';
import { legacyProjectCopyToCanvasCopy } from './copy';
import type { CreativeStudioProjectsCopy } from './copy';
import type { CreativeStudioProjectSummary } from './types';

interface CreativeStudioProjectCardProps {
  project: CreativeStudioProjectSummary;
  copy: CreativeStudioProjectsCopy;
  language?: string;
  selected: boolean;
  editing: boolean;
  editingTitle: string;
  disabled?: boolean;
  exportDisabled?: boolean;
  archiveUnavailableMessage?: string;
  onOpen: (project: CreativeStudioProjectSummary) => void;
  onToggleSelected: (
    project: CreativeStudioProjectSummary,
    selected: boolean
  ) => void;
  onStartRename: (project: CreativeStudioProjectSummary) => void;
  onEditingTitleChange: (title: string) => void;
  onSaveRename: () => void;
  onCancelRename: () => void;
  onExport: (project: CreativeStudioProjectSummary) => void;
  onDelete: (project: CreativeStudioProjectSummary) => void;
}

/** @deprecated Compatibility adapter over CreativeStudioCanvasCard. */
const CreativeStudioProjectCard: React.FC<
  CreativeStudioProjectCardProps
> = ({
  project,
  copy,
  onOpen,
  onToggleSelected,
  onStartRename,
  onExport,
  onDelete,
  ...props
}) => {
  const canvas = {
    canvasId: project.id,
    title: project.title,
    revision: '0',
    nodeCount: project.nodeCount,
    connectionCount: project.connectionCount,
    createdAt: project.createdAt,
    updatedAt: project.updatedAt,
  };
  const canvasCopy = legacyProjectCopyToCanvasCopy(copy);

  return (
    <CreativeStudioCanvasCard
      {...props}
      canvas={canvas}
      copy={canvasCopy}
      onOpen={() => onOpen(project)}
      onToggleSelected={(_canvas, selected) =>
        onToggleSelected(project, selected)
      }
      onStartRename={() => onStartRename(project)}
      onExport={() => onExport(project)}
      onDelete={() => onDelete(project)}
    />
  );
};

export default CreativeStudioProjectCard;
