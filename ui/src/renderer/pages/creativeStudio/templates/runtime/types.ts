/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  CreativeTaskStatus,
  CreativeTaskPort,
} from '../../tasks';
import type {
  CreativeTemplateDefinitionV1,
  CreativeTemplateInputValue,
  CreativeTemplateRunAggregateV1,
} from '../domain';
import type { CreativeTemplateRunApi } from '../services';

export interface StartCreativeTemplateRun {
  template: CreativeTemplateDefinitionV1;
  inputs: CreativeTemplateInputValue[];
  referenceAssetIds: string[];
}

export interface ReviewCreativeTemplateDraft {
  id: string;
  title: string;
  prompt: string;
  reviewNote?: string | null;
}

export type CreativeTemplateRunActivityState =
  | 'submitting'
  | 'executing'
  | 'awaiting-review'
  | 'paused'
  | 'cancelling';

export interface CreativeTemplateRunActivity {
  state: CreativeTemplateRunActivityState;
  taskStatuses: Readonly<Record<string, CreativeTaskStatus>>;
  error: string | null;
}

export interface CreativeTemplateRuntimeSnapshot {
  loading: boolean;
  loadError: string | null;
  runs: readonly CreativeTemplateRunAggregateV1[];
  activities: Readonly<Record<string, CreativeTemplateRunActivity>>;
}

export interface CreativeTemplateTextAssetReader {
  read(assetId: string, signal?: AbortSignal): Promise<string>;
}

export interface CreativeTemplateRuntimeDependencies {
  runs: CreativeTemplateRunApi;
  tasks: CreativeTaskPort;
  textAssets: CreativeTemplateTextAssetReader;
  createId(): string;
  now(): number;
  pollIntervalMs?: number;
  pollMaxWaitMs?: number;
}

export interface CreativeTemplateRunner {
  subscribe(listener: () => void): () => void;
  getSnapshot(): CreativeTemplateRuntimeSnapshot;
  load(): Promise<void>;
  start(input: StartCreativeTemplateRun): Promise<CreativeTemplateRunAggregateV1>;
  resume(templateRunId: string): Promise<CreativeTemplateRunAggregateV1>;
  review(
    templateRunId: string,
    drafts: readonly ReviewCreativeTemplateDraft[]
  ): Promise<CreativeTemplateRunAggregateV1>;
  cancel(templateRunId: string): Promise<CreativeTemplateRunAggregateV1>;
}

export type CreativeTemplateRunRuntimeErrorCode =
  | 'invalid-plan'
  | 'planner-output'
  | 'task-failed'
  | 'task-cancelled'
  | 'asset-response';

export class CreativeTemplateRunRuntimeError extends Error {
  readonly code: CreativeTemplateRunRuntimeErrorCode;

  constructor(code: CreativeTemplateRunRuntimeErrorCode, message: string) {
    super(message);
    this.name = 'CreativeTemplateRunRuntimeError';
    this.code = code;
  }
}

export class CreativeTemplateTextAssetHttpError extends Error {
  readonly status: number;

  constructor(status: number, message: string) {
    super(message);
    this.name = 'CreativeTemplateTextAssetHttpError';
    this.status = status;
  }
}
