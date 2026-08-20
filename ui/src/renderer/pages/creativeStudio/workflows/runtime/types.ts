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
  WorkflowDefinitionV1,
  WorkflowInputValue,
  WorkflowRunAggregateV1,
} from '../domain';
import type { CreativeWorkflowRunApi } from '../services';

export interface StartCreativeWorkflowRun {
  workflow: WorkflowDefinitionV1;
  inputs: WorkflowInputValue[];
  referenceAssetIds: string[];
}

export interface ReviewCreativeWorkflowDraft {
  id: string;
  title: string;
  prompt: string;
  reviewNote?: string | null;
}

export type CreativeWorkflowRunActivityState =
  | 'submitting'
  | 'executing'
  | 'awaiting-review'
  | 'paused'
  | 'cancelling';

export interface CreativeWorkflowRunActivity {
  state: CreativeWorkflowRunActivityState;
  taskStatuses: Readonly<Record<string, CreativeTaskStatus>>;
  error: string | null;
}

export interface CreativeWorkflowRuntimeSnapshot {
  loading: boolean;
  loadError: string | null;
  runs: readonly WorkflowRunAggregateV1[];
  activities: Readonly<Record<string, CreativeWorkflowRunActivity>>;
}

export interface WorkflowTextAssetReader {
  read(assetId: string, signal?: AbortSignal): Promise<string>;
}

export interface CreativeWorkflowRuntimeDependencies {
  runs: CreativeWorkflowRunApi;
  tasks: CreativeTaskPort;
  textAssets: WorkflowTextAssetReader;
  createId(): string;
  now(): number;
  pollIntervalMs?: number;
  pollMaxWaitMs?: number;
}

export interface CreativeWorkflowRunner {
  subscribe(listener: () => void): () => void;
  getSnapshot(): CreativeWorkflowRuntimeSnapshot;
  load(): Promise<void>;
  start(input: StartCreativeWorkflowRun): Promise<WorkflowRunAggregateV1>;
  resume(runId: string): Promise<WorkflowRunAggregateV1>;
  review(
    runId: string,
    drafts: readonly ReviewCreativeWorkflowDraft[]
  ): Promise<WorkflowRunAggregateV1>;
  cancel(runId: string): Promise<WorkflowRunAggregateV1>;
}

export type WorkflowRunRuntimeErrorCode =
  | 'invalid-plan'
  | 'planner-output'
  | 'task-failed'
  | 'task-cancelled'
  | 'asset-response';

export class WorkflowRunRuntimeError extends Error {
  readonly code: WorkflowRunRuntimeErrorCode;

  constructor(code: WorkflowRunRuntimeErrorCode, message: string) {
    super(message);
    this.name = 'WorkflowRunRuntimeError';
    this.code = code;
  }
}

export class WorkflowTextAssetHttpError extends Error {
  readonly status: number;

  constructor(status: number, message: string) {
    super(message);
    this.name = 'WorkflowTextAssetHttpError';
    this.status = status;
  }
}
