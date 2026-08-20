/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ReactNode } from 'react';

export type VideoWorkbenchLayout = 'side' | 'bottom';
export type VideoReferenceKind = 'image' | 'video' | 'audio';

export interface VideoWorkbenchReference {
  id: string;
  kind: VideoReferenceKind;
  name: string;
  /** Optional real thumbnail supplied by the asset layer. */
  previewUrl?: string;
}

export interface VideoWorkbenchChoice {
  value: string;
  label: string;
}

interface VideoWorkbenchTaskBase {
  id: string;
  prompt: string;
  createdAtLabel: string;
  modelLabel: string;
  resolutionLabel: string;
  sizeLabel: string;
  durationLabel: string;
  taskCount: number;
}

export interface RunningVideoWorkbenchTask extends VideoWorkbenchTaskBase {
  status: 'running';
  progress?: number;
  elapsedLabel?: string;
}

export interface SuccessfulVideoWorkbenchTask extends VideoWorkbenchTaskBase {
  status: 'success';
  /** A successful task must provide a real playable URL; the view never fakes one. */
  videoUrl: string;
  posterUrl?: string;
  mediaMetaLabel?: string;
}

export interface FailedVideoWorkbenchTask extends VideoWorkbenchTaskBase {
  status: 'failed';
  error: string;
  errorDetail?: string;
}

export type VideoWorkbenchTask =
  | RunningVideoWorkbenchTask
  | SuccessfulVideoWorkbenchTask
  | FailedVideoWorkbenchTask;

export interface VideoWorkbenchProps {
  layout: VideoWorkbenchLayout;
  onLayoutChange: (layout: VideoWorkbenchLayout) => void;

  prompt: string;
  onPromptChange: (value: string) => void;
  onGenerate: () => void;
  generating?: boolean;
  submitDisabled?: boolean;

  references: readonly VideoWorkbenchReference[];
  onAddReferences: () => void;
  onRemoveReference: (referenceId: string) => void;
  onMoveReference?: (referenceId: string, direction: -1 | 1) => void;

  /** The Creative Studio model catalog owns model selection and state copy. */
  modelSlot: ReactNode;

  resolution: string;
  resolutionOptions: readonly VideoWorkbenchChoice[];
  onResolutionChange: (value: string) => void;
  size: string;
  sizeOptions: readonly VideoWorkbenchChoice[];
  onSizeChange: (value: string) => void;
  duration: string;
  durationOptions: readonly VideoWorkbenchChoice[];
  onDurationChange: (value: string) => void;
  taskCount: number;
  onTaskCountChange: (value: number) => void;
  onOpenParameters: () => void;
  onOpenPromptLibrary?: () => void;

  tasks: readonly VideoWorkbenchTask[];
  selectedTaskIds: readonly string[];
  onSelectedTaskIdsChange: (ids: readonly string[]) => void;
  onDeleteTasks: (ids: readonly string[]) => void;
  onNewSession?: () => void;
  onLoadTask?: (taskId: string) => void;
  onRetryTask?: (taskId: string) => void;
  onInspectTask?: (taskId: string) => void;
  onCopyPrompt?: (prompt: string) => void;
  onDownloadTask?: (taskId: string) => void;

  className?: string;
}

export type VideoResultsState = 'empty' | 'running' | 'success' | 'failed' | 'mixed';
