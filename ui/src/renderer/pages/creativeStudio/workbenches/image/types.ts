/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ReactNode } from 'react';

export type ImageWorkbenchLayout = 'side' | 'bottom';
export type ImageWorkbenchInterfaceMode = 'images' | 'responses';
export type ImageWorkbenchQuality = 'auto' | 'high' | 'medium' | 'low';
export type ImageWorkbenchTaskState =
  | 'idle'
  | 'queued'
  | 'running'
  | 'succeeded'
  | 'failed'
  | 'canceled';

/** Exact NomiFun model identity. Display labels never become request identity. */
export interface ImageWorkbenchModelIdentity {
  providerId: string;
  model: string;
}

export interface ImageWorkbenchModelOption extends ImageWorkbenchModelIdentity {
  label: string;
  providerLabel?: string;
  disabled?: boolean;
}

export interface ImageWorkbenchAspectRatioOption {
  value: string;
  label: string;
  width: number | null;
  height: number | null;
  disabled?: boolean;
}

export interface ImageWorkbenchSettings {
  model: ImageWorkbenchModelIdentity | null;
  interfaceMode: ImageWorkbenchInterfaceMode;
  quality: ImageWorkbenchQuality;
  width: number | null;
  height: number | null;
  aspectRatio: string;
  count: number;
}

/** References are always real assets supplied by the caller; the UI invents no media. */
export interface ImageWorkbenchReference {
  id: string;
  name: string;
  previewUrl: string;
}

interface ImageWorkbenchResultBase {
  id: string;
  /** Runtime task identity remains distinct from the generated asset identity. */
  taskId: string;
  prompt: string;
  model: ImageWorkbenchModelIdentity;
  modelLabel: string;
  createdAtLabel?: string;
  durationLabel?: string;
  /** False when legacy inputs or the exact model are no longer available. */
  retryable?: boolean;
  /** Only terminal tasks may be retired from durable history. */
  deletable?: boolean;
}

export interface ImageWorkbenchQueuedResult extends ImageWorkbenchResultBase {
  status: 'queued';
  statusLabel?: string;
}

export interface ImageWorkbenchRunningResult extends ImageWorkbenchResultBase {
  status: 'running';
  progress?: number;
  statusLabel?: string;
}

export interface ImageWorkbenchSucceededResult extends ImageWorkbenchResultBase {
  status: 'succeeded';
  /** One task card retains every output in authoritative result order. */
  outputs: readonly {
    assetId: string;
    imageUrl: string;
    alt: string;
    width?: number;
    height?: number;
    sizeLabel?: string;
  }[];
}

export interface ImageWorkbenchFailedResult extends ImageWorkbenchResultBase {
  status: 'failed';
  errorMessage: string;
  errorDetail?: string;
}

export interface ImageWorkbenchCanceledResult extends ImageWorkbenchResultBase {
  status: 'canceled';
  message?: string;
}

export type ImageWorkbenchResult =
  | ImageWorkbenchQueuedResult
  | ImageWorkbenchRunningResult
  | ImageWorkbenchSucceededResult
  | ImageWorkbenchFailedResult
  | ImageWorkbenchCanceledResult;

export interface ImageWorkbenchTaskSummary {
  state: ImageWorkbenchTaskState;
  pendingCount: number;
  message?: string;
}

export interface ImageWorkbenchProps {
  layout: ImageWorkbenchLayout;
  prompt: string;
  references: readonly ImageWorkbenchReference[];
  settings: ImageWorkbenchSettings;
  modelOptions: readonly ImageWorkbenchModelOption[];
  /** Optional catalog-owned selector with explicit loading/error/empty states. */
  modelSlot?: ReactNode;
  results: readonly ImageWorkbenchResult[];
  selectedResultIds: readonly string[];
  task: ImageWorkbenchTaskSummary;
  disabled?: boolean;
  uploadingReferenceCount?: number;
  onLayoutChange(layout: ImageWorkbenchLayout): void;
  onPromptChange(prompt: string): void;
  onPastePrompt?(): void;
  onClearPrompt?(): void;
  onOpenPromptLibrary?(): void;
  onPasteReferences?(): void;
  onUploadReferences?(): void;
  onChooseReferences?(): void;
  onRemoveReference(referenceId: string): void;
  onModelChange(model: ImageWorkbenchModelIdentity | null): void;
  onInterfaceModeChange(mode: ImageWorkbenchInterfaceMode): void;
  onQualityChange(quality: ImageWorkbenchQuality): void;
  onDimensionsChange(dimensions: { width: number | null; height: number | null }): void;
  onAspectRatioChange(option: ImageWorkbenchAspectRatioOption): void;
  onCountChange(count: number): void;
  onGenerate(): void;
  onResultSelectionChange(resultIds: string[]): void;
  onDeleteResult?(resultId: string): void;
  onDeleteSelected?(resultIds: string[]): void;
  onRetryResult?(resultId: string): void;
  onLoadResult?(taskId: string): void;
  onCancelTask?(taskId: string): void;
  historyLoading?: boolean;
  historyError?: string;
  historyLoadingMore?: boolean;
  historyHasMore?: boolean;
  onLoadMoreResults?(): void;
}

export const DEFAULT_IMAGE_WORKBENCH_ASPECT_RATIOS: readonly ImageWorkbenchAspectRatioOption[] = [
  { value: '1:1', label: '1:1', width: 1024, height: 1024 },
  { value: '3:2', label: '3:2', width: 1536, height: 1024 },
  { value: '2:3', label: '2:3', width: 1024, height: 1536 },
  { value: '4:3', label: '4:3', width: 1024, height: 768 },
  { value: '3:4', label: '3:4', width: 768, height: 1024 },
  { value: '16:9', label: '16:9', width: 1920, height: 1080 },
  { value: '9:16', label: '9:16', width: 1080, height: 1920 },
  { value: '21:9', label: '21:9', width: 1568, height: 672 },
  { value: '2048x2048', label: '1:1 · 2K', width: 2048, height: 2048 },
  { value: '2048x1152', label: '16:9 · 2K', width: 2048, height: 1152 },
  { value: '1152x2048', label: '9:16 · 2K', width: 1152, height: 2048 },
  { value: '3840x2160', label: '16:9 · 4K', width: 3840, height: 2160 },
  { value: '2160x3840', label: '9:16 · 4K', width: 2160, height: 3840 },
  { value: 'auto', label: '自动', width: null, height: null },
];

export const IMAGE_WORKBENCH_QUALITY_OPTIONS: readonly {
  value: ImageWorkbenchQuality;
  label: string;
}[] = [
  { value: 'auto', label: '自动' },
  { value: 'high', label: '高' },
  { value: 'medium', label: '中' },
  { value: 'low', label: '低' },
];

export function imageWorkbenchModelKey(model: ImageWorkbenchModelIdentity): string {
  return JSON.stringify([model.providerId, model.model]);
}

export function parseImageWorkbenchModelKey(
  key: string,
  options: readonly ImageWorkbenchModelOption[]
): ImageWorkbenchModelIdentity | null {
  const option = options.find((candidate) => imageWorkbenchModelKey(candidate) === key);
  return option ? { providerId: option.providerId, model: option.model } : null;
}

export function nextImageWorkbenchSelection(
  selectedIds: readonly string[],
  resultId: string,
  selected: boolean
): string[] {
  return selected
    ? [...new Set([...selectedIds, resultId])]
    : selectedIds.filter((candidate) => candidate !== resultId);
}
