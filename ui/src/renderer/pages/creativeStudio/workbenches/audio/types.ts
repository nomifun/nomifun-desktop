/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ReactNode } from 'react';

export type AudioWorkbenchTaskState =
  | 'idle'
  | 'queued'
  | 'running'
  | 'succeeded'
  | 'failed'
  | 'canceled';

/** Exact NomiFun catalog coordinate for the `speech_synthesis` task. */
export interface AudioWorkbenchModelIdentity {
  providerId: string;
  model: string;
}

export interface AudioWorkbenchOption {
  value: string;
  label: string;
  disabled?: boolean;
}

/**
 * Product intent only. The integration adapter decides which optional values
 * the selected provider protocol can carry; this object is not an API body.
 */
export interface AudioWorkbenchValue {
  text: string;
  instructions: string;
  voice: string;
  format: string;
  speed: number;
  model: AudioWorkbenchModelIdentity | null;
}

/** References are real caller-owned assets; the workbench invents no audio. */
export interface AudioWorkbenchReference {
  assetId: string;
  name: string;
  mimeType?: string;
  durationMs?: number;
  sizeBytes?: number;
}

interface AudioWorkbenchResultBase {
  id: string;
  taskId: string;
  title: string;
  text?: string;
  modelLabel?: string;
  formatLabel?: string;
  createdAtLabel?: string;
}

export interface AudioWorkbenchQueuedResult extends AudioWorkbenchResultBase {
  status: 'queued';
  statusLabel?: string;
}

export interface AudioWorkbenchRunningResult extends AudioWorkbenchResultBase {
  status: 'running';
  progress?: number;
  statusLabel?: string;
}

export interface AudioWorkbenchSucceededResult extends AudioWorkbenchResultBase {
  status: 'succeeded';
  /** Stable generated asset identity; bytes and URLs stay outside this UI. */
  assetId: string;
  durationMs?: number;
  sizeBytes?: number;
}

export interface AudioWorkbenchFailedResult extends AudioWorkbenchResultBase {
  status: 'failed';
  errorMessage: string;
}

export interface AudioWorkbenchCanceledResult extends AudioWorkbenchResultBase {
  status: 'canceled';
  message?: string;
}

export type AudioWorkbenchResult =
  | AudioWorkbenchQueuedResult
  | AudioWorkbenchRunningResult
  | AudioWorkbenchSucceededResult
  | AudioWorkbenchFailedResult
  | AudioWorkbenchCanceledResult;

export interface AudioWorkbenchTaskSummary {
  state: AudioWorkbenchTaskState;
  taskId?: string;
  progress?: number;
  message?: string;
  errorMessage?: string;
}

export interface AudioWorkbenchFieldSupport {
  voice: boolean;
  format: boolean;
  speed: boolean;
  instructions: boolean;
  references: boolean;
}

export interface AudioWorkbenchSpeedRange {
  min: number;
  max: number;
  step: number;
}

export interface AudioWorkbenchProps {
  value: AudioWorkbenchValue;
  /** Inject the canonical NomiFun `speech_synthesis` model selector here. */
  modelSlot: ReactNode;
  voiceOptions: readonly AudioWorkbenchOption[];
  formatOptions: readonly AudioWorkbenchOption[];
  references: readonly AudioWorkbenchReference[];
  results: readonly AudioWorkbenchResult[];
  task: AudioWorkbenchTaskSummary;
  playingResultId?: string | null;
  disabled?: boolean;
  maxTextLength?: number;
  speedRange?: AudioWorkbenchSpeedRange;
  fieldSupport?: Partial<AudioWorkbenchFieldSupport>;
  referenceRequired?: boolean;
  onValueChange(value: AudioWorkbenchValue): void;
  onChooseReferences?(): void;
  onRemoveReference(referenceAssetId: string): void;
  onGenerate(value: AudioWorkbenchValue): void;
  onCancel?(): void;
  onRetry?(): void;
  onPlaybackChange(result: AudioWorkbenchSucceededResult, shouldPlay: boolean): void;
  onDownloadResult(result: AudioWorkbenchSucceededResult): void;
  onInsertResult(result: AudioWorkbenchSucceededResult): void;
  onRetryResult?(result: AudioWorkbenchFailedResult | AudioWorkbenchCanceledResult): void;
}

export const DEFAULT_AUDIO_WORKBENCH_SPEED_RANGE: AudioWorkbenchSpeedRange = {
  min: 0.25,
  max: 4,
  step: 0.05,
};

export const DEFAULT_AUDIO_WORKBENCH_FIELD_SUPPORT: AudioWorkbenchFieldSupport = {
  voice: true,
  format: true,
  // The generic NomiFun `/api/tts` wire does not carry these fields. A future
  // task adapter must opt them in only after resolving protocol support.
  speed: false,
  instructions: false,
  references: false,
};

export const isAudioWorkbenchBusy = (state: AudioWorkbenchTaskState): boolean =>
  state === 'queued' || state === 'running';

export const clampAudioWorkbenchSpeed = (
  value: number,
  range: AudioWorkbenchSpeedRange = DEFAULT_AUDIO_WORKBENCH_SPEED_RANGE
): number => {
  const finite = Number.isFinite(value) ? value : 1;
  return Math.min(range.max, Math.max(range.min, Number(finite.toFixed(2))));
};

export const canGenerateAudioWorkbench = (
  value: AudioWorkbenchValue,
  taskState: AudioWorkbenchTaskState,
  referenceCount: number,
  options: { disabled?: boolean; maxTextLength?: number; referenceRequired?: boolean } = {}
): boolean => {
  const textLength = Array.from(value.text).length;
  return (
    !options.disabled &&
    !isAudioWorkbenchBusy(taskState) &&
    value.model !== null &&
    value.text.trim().length > 0 &&
    textLength <= (options.maxTextLength ?? 4096) &&
    (!options.referenceRequired || referenceCount > 0)
  );
};
