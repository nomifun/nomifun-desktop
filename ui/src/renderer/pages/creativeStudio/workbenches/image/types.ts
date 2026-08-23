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
  /** Catalog protocol metadata used to select a provider-safe size policy. */
  platform?: string;
  protocol?: string;
  disabled?: boolean;
}

export interface ImageWorkbenchAspectRatioOption {
  value: string;
  label: string;
  width: number | null;
  height: number | null;
  /**
   * Exact provider-native size value. This is intentionally separate from
   * width/height because some providers use a different ordering or a fixed
   * enumeration for their wire contract.
   */
  requestSize?: string;
  disabled?: boolean;
}

export interface ImageWorkbenchSizePolicy {
  options: readonly ImageWorkbenchAspectRatioOption[];
  allowCustomDimensions: boolean;
  maxCount: number;
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
  aspectRatioOptions?: readonly ImageWorkbenchAspectRatioOption[];
  dimensionsDisabled?: boolean;
  maxCount?: number;
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
  onCancelTask?(taskId: string): void;
  historyLoading?: boolean;
  historyError?: string;
  historyLoadingMore?: boolean;
  historyHasMore?: boolean;
  onLoadMoreResults?(): void;
}

export const DEFAULT_IMAGE_WORKBENCH_ASPECT_RATIOS: readonly ImageWorkbenchAspectRatioOption[] = [
  { value: '1:1', label: '1:1', width: 1024, height: 1024, requestSize: '1024x1024' },
  { value: '3:2', label: '3:2', width: 1536, height: 1024, requestSize: '1536x1024' },
  { value: '2:3', label: '2:3', width: 1024, height: 1536, requestSize: '1024x1536' },
  { value: '4:3', label: '4:3', width: 1024, height: 768, requestSize: '1024x768' },
  { value: '3:4', label: '3:4', width: 768, height: 1024, requestSize: '768x1024' },
  { value: '16:9', label: '16:9', width: 1920, height: 1080, requestSize: '1920x1080' },
  { value: '9:16', label: '9:16', width: 1080, height: 1920, requestSize: '1080x1920' },
  { value: '21:9', label: '21:9', width: 1568, height: 672, requestSize: '1568x672' },
  { value: '2048x2048', label: '1:1 · 2K', width: 2048, height: 2048, requestSize: '2048x2048' },
  { value: '2048x1152', label: '16:9 · 2K', width: 2048, height: 1152, requestSize: '2048x1152' },
  { value: '1152x2048', label: '9:16 · 2K', width: 1152, height: 2048, requestSize: '1152x2048' },
  { value: '3840x2160', label: '16:9 · 4K', width: 3840, height: 2160, requestSize: '3840x2160' },
  { value: '2160x3840', label: '9:16 · 4K', width: 2160, height: 3840, requestSize: '2160x3840' },
  { value: 'auto', label: '自动', width: null, height: null },
];

/**
 * StepFun's image API uses a strict size enum. For step-image-edit-2 the
 * non-square wire value is height x width, while the workbench always keeps
 * width/height in the user-facing order.
 */
const STEPFUN_IMAGE_EDIT_2_ASPECT_RATIOS: readonly ImageWorkbenchAspectRatioOption[] = [
  {
    value: '1:1',
    label: '1:1',
    width: 1024,
    height: 1024,
    requestSize: '1024x1024',
  },
  {
    value: '16:9',
    label: '16:9',
    width: 1360,
    height: 768,
    requestSize: '768x1360',
  },
  {
    value: '4:3',
    label: '4:3',
    width: 1184,
    height: 896,
    requestSize: '896x1184',
  },
  {
    value: '9:16',
    label: '9:16',
    width: 768,
    height: 1360,
    requestSize: '1360x768',
  },
  {
    value: '3:4',
    label: '3:4',
    width: 896,
    height: 1184,
    requestSize: '1184x896',
  },
  { value: 'auto', label: '自动', width: null, height: null },
];

const STEPFUN_STEP_2X_LARGE_ASPECT_RATIOS: readonly ImageWorkbenchAspectRatioOption[] = [
  {
    value: '1:1-256',
    label: '1:1 · 256',
    width: 256,
    height: 256,
    requestSize: '256x256',
  },
  {
    value: '1:1-512',
    label: '1:1 · 512',
    width: 512,
    height: 512,
    requestSize: '512x512',
  },
  {
    value: '1:1',
    label: '1:1',
    width: 1024,
    height: 1024,
    requestSize: '1024x1024',
  },
  {
    value: '16:9',
    label: '16:9',
    width: 1280,
    height: 800,
    requestSize: '1280x800',
  },
  {
    value: '9:16',
    label: '9:16',
    width: 800,
    height: 1280,
    requestSize: '800x1280',
  },
  { value: 'auto', label: '自动', width: null, height: null },
];

const STEPFUN_UNKNOWN_MODEL_ASPECT_RATIOS: readonly ImageWorkbenchAspectRatioOption[] = [
  DEFAULT_IMAGE_WORKBENCH_ASPECT_RATIOS[0],
  DEFAULT_IMAGE_WORKBENCH_ASPECT_RATIOS[DEFAULT_IMAGE_WORKBENCH_ASPECT_RATIOS.length - 1],
];

export function imageWorkbenchSizePolicyForModel(
  model:
    | Pick<ImageWorkbenchModelOption, 'model' | 'platform' | 'protocol'>
    | null
    | undefined
): ImageWorkbenchSizePolicy {
  if (!model) {
    return {
      options: DEFAULT_IMAGE_WORKBENCH_ASPECT_RATIOS,
      allowCustomDimensions: true,
      maxCount: 10,
    };
  }

  const protocol = model.protocol?.trim().toLowerCase();
  const modelId = model.model.trim().toLowerCase();
  const isStepFunImages = protocol === 'stepfun.images';
  if (!isStepFunImages) {
    return {
      options: DEFAULT_IMAGE_WORKBENCH_ASPECT_RATIOS,
      allowCustomDimensions: true,
      maxCount: 10,
    };
  }

  if (modelId === 'step-image-edit-2') {
    return {
      options: STEPFUN_IMAGE_EDIT_2_ASPECT_RATIOS,
      allowCustomDimensions: false,
      maxCount: 1,
    };
  }
  if (modelId === 'step-2x-large') {
    return {
      options: STEPFUN_STEP_2X_LARGE_ASPECT_RATIOS,
      allowCustomDimensions: false,
      maxCount: 1,
    };
  }

  // Unknown future StepFun image models fail closed in the UI until their
  // documented size contract is added, while square/automatic generation
  // remains available.
  return {
    options: STEPFUN_UNKNOWN_MODEL_ASPECT_RATIOS,
    allowCustomDimensions: false,
    maxCount: 1,
  };
}

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
