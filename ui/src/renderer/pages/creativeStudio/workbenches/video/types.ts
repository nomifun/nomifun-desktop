/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ReactNode } from 'react';

export type VideoWorkbenchLayout = 'side' | 'bottom';
export type VideoReferenceKind = 'image' | 'video' | 'audio';

/** Exact NomiFun catalog coordinate for the `video_generation` task. */
export interface VideoWorkbenchModelIdentity {
  providerId: string;
  model: string;
}

export interface VideoWorkbenchReference {
  id: string;
  kind: VideoReferenceKind;
  name: string;
  /** Optional real thumbnail supplied by the asset layer. */
  previewUrl?: string;
  /** Original media is kept separate so a video or audio file never becomes an image source. */
  originalUrl?: string;
}

export interface VideoWorkbenchChoice {
  value: string;
  label: string;
}

interface VideoWorkbenchTaskBase {
  hasDeletedInputs?: boolean;
  id: string;
  /** Runtime task identity remains distinct from the generated asset identity. */
  taskId: string;
  prompt: string;
  createdAtLabel: string;
  model: VideoWorkbenchModelIdentity;
  modelLabel: string;
  resolutionLabel: string;
  sizeLabel: string;
  durationLabel: string;
  taskCount: number;
  /** False when legacy inputs or the exact model are no longer available. */
  retryable?: boolean;
  /** Only terminal tasks may be retired from durable history. */
  deletable?: boolean;
}

export interface QueuedVideoWorkbenchTask extends VideoWorkbenchTaskBase {
  status: 'queued';
  statusLabel?: string;
}

export interface RunningVideoWorkbenchTask extends VideoWorkbenchTaskBase {
  status: 'running';
  progress?: number;
  elapsedLabel?: string;
}

export interface SucceededVideoWorkbenchTask extends VideoWorkbenchTaskBase {
  availability?: import('../../assets').CreativeAssetAvailability;
  status: 'succeeded';
  /** Stable generated asset identity; the URL is a caller-resolved presentation detail. */
  assetId: string;
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

export interface CanceledVideoWorkbenchTask extends VideoWorkbenchTaskBase {
  status: 'canceled';
  message?: string;
}

export type VideoWorkbenchTask =
  | QueuedVideoWorkbenchTask
  | RunningVideoWorkbenchTask
  | SucceededVideoWorkbenchTask
  | FailedVideoWorkbenchTask
  | CanceledVideoWorkbenchTask;

export interface VideoWorkbenchProps {
  layout: VideoWorkbenchLayout;
  onLayoutChange: (layout: VideoWorkbenchLayout) => void;

  prompt: string;
  onPromptChange: (value: string) => void;
  onGenerate: () => void;
  generating?: boolean;
  submitDisabled?: boolean;

  references: readonly VideoWorkbenchReference[];
  addReferenceLabel?: string;
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
  onDeleteTasks?: (ids: readonly string[]) => void;
  onNewSession?: () => void;
  onLoadTask?: (taskId: string) => void;
  onRetryTask?: (taskId: string) => void;
  onCancelTask?: (taskId: string) => void;
  onInspectTask?: (taskId: string) => void;
  onCopyPrompt?: (prompt: string) => void;
  onDownloadTask?: (taskId: string) => void;
  historyLoading?: boolean;
  historyError?: string;
  historyLoadingMore?: boolean;
  historyHasMore?: boolean;
  onLoadMoreTasks?: () => void;

  className?: string;
}

export type VideoResultsState =
  | 'empty'
  | 'queued'
  | 'running'
  | 'succeeded'
  | 'failed'
  | 'canceled'
  | 'mixed';
