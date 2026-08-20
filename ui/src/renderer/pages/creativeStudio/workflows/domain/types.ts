/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

export type WorkflowId = string;
export type WorkflowVariableId = string;
export type WorkflowTemplateId = string;
export type WorkflowStepId = string;
export type WorkflowRunRequestId = string;
export type WorkflowPromptDraftId = string;
export type WorkflowAssetId = string;
export type WorkflowTaskId = string;
export type WorkflowHistoryReferenceId = string;

export type WorkflowVisibility = 'private' | 'public';

/** Typed, user-facing catalog information. There is no opaque metadata bag. */
export interface WorkflowMetadata {
  name: string;
  description: string;
  category: string;
  visibility: WorkflowVisibility;
  tags: string[];
  createdAt: number;
  updatedAt: number;
}

interface WorkflowVariableBase {
  id: WorkflowVariableId;
  key: string;
  label: string;
  description: string;
  required: boolean;
}

export interface WorkflowTextVariable extends WorkflowVariableBase {
  type: 'text' | 'multiline-text';
  defaultValue: string | null;
  placeholder: string;
  minLength: number;
  maxLength: number;
}

export interface WorkflowNumberVariable extends WorkflowVariableBase {
  type: 'number';
  defaultValue: number | null;
  minimum: number | null;
  maximum: number | null;
  step: number | null;
}

export interface WorkflowBooleanVariable extends WorkflowVariableBase {
  type: 'boolean';
  defaultValue: boolean;
}

export interface WorkflowChoiceVariable extends WorkflowVariableBase {
  type: 'choice';
  defaultValue: string | null;
  options: string[];
}

export interface WorkflowImageVariable extends WorkflowVariableBase {
  type: 'image';
  defaultAssetId: WorkflowAssetId | null;
}

export interface WorkflowImageSeriesVariable extends WorkflowVariableBase {
  type: 'image-series';
  defaultAssetIds: WorkflowAssetId[];
  minItems: number;
  maxItems: number;
}

export type WorkflowVariable =
  | WorkflowTextVariable
  | WorkflowNumberVariable
  | WorkflowBooleanVariable
  | WorkflowChoiceVariable
  | WorkflowImageVariable
  | WorkflowImageSeriesVariable;

export type WorkflowTemplateSegment =
  | { kind: 'text'; text: string }
  | { kind: 'variable'; variableId: WorkflowVariableId };

export interface WorkflowTemplate {
  id: WorkflowTemplateId;
  name: string;
  segments: WorkflowTemplateSegment[];
}

export type WorkflowOutputPlan =
  | { kind: 'single-image' }
  | {
      kind: 'multi-image-series';
      targetCount: number;
      concurrency: number;
      reviewRequired: boolean;
    };

interface WorkflowStepBase {
  id: WorkflowStepId;
  name: string;
  dependsOn: WorkflowStepId[];
  enabled: boolean;
}

export interface WorkflowRenderTemplateStep extends WorkflowStepBase {
  kind: 'render-template';
  templateId: WorkflowTemplateId;
}

export interface WorkflowDraftPromptsStep extends WorkflowStepBase {
  kind: 'draft-prompts';
  templateId: WorkflowTemplateId;
  planning: WorkflowPromptPlanningSettings;
}

export interface WorkflowTextModelBinding {
  providerId: string;
  model: string;
  task: 'chat';
}

/** Exact NomiFun Chat model used to turn one rendered brief into reviewed prompts. */
export interface WorkflowPromptPlanningSettings {
  model: WorkflowTextModelBinding | null;
  instruction: string;
  maxTokens: number;
}

export type WorkflowImagePromptSource =
  | { kind: 'template'; templateId: WorkflowTemplateId }
  | { kind: 'prompt-drafts'; stepId: WorkflowStepId };

export type WorkflowImageTask = 'image_generation' | 'image_edit';

export interface WorkflowImageModelBinding {
  providerId: string;
  model: string;
  task: WorkflowImageTask;
}

export type WorkflowImageQuality = 'auto' | 'high' | 'medium' | 'low';

/** Provider-neutral settings persisted with the workflow definition. */
export interface WorkflowImageGenerationSettings {
  model: WorkflowImageModelBinding | null;
  quality: WorkflowImageQuality;
  width: number;
  height: number;
  imagesPerPrompt: number;
}

export interface WorkflowGenerateImagesStep extends WorkflowStepBase {
  kind: 'generate-images';
  promptSource: WorkflowImagePromptSource;
  referenceVariableIds: WorkflowVariableId[];
  generation: WorkflowImageGenerationSettings;
}

export interface WorkflowRecordHistoryStep extends WorkflowStepBase {
  kind: 'record-history';
  sourceStepIds: WorkflowStepId[];
}

export type WorkflowStep =
  | WorkflowRenderTemplateStep
  | WorkflowDraftPromptsStep
  | WorkflowGenerateImagesStep
  | WorkflowRecordHistoryStep;

export interface WorkflowDefinitionV1 {
  id: WorkflowId;
  revision: number;
  metadata: WorkflowMetadata;
  output: WorkflowOutputPlan;
  variables: WorkflowVariable[];
  templates: WorkflowTemplate[];
  steps: WorkflowStep[];
}

export type WorkflowInputValue =
  | { variableId: WorkflowVariableId; type: 'text' | 'multiline-text' | 'choice'; value: string }
  | { variableId: WorkflowVariableId; type: 'number'; value: number }
  | { variableId: WorkflowVariableId; type: 'boolean'; value: boolean }
  | { variableId: WorkflowVariableId; type: 'image'; assetId: WorkflowAssetId | null }
  | { variableId: WorkflowVariableId; type: 'image-series'; assetIds: WorkflowAssetId[] };

export interface WorkflowRunRequest {
  id: WorkflowRunRequestId;
  idempotencyKey: string;
  workflowId: WorkflowId;
  workflowRevision: number;
  requestedAt: number;
  output: WorkflowOutputPlan;
  inputs: WorkflowInputValue[];
}

export type WorkflowPromptDraftStatus = 'pending-review' | 'approved' | 'rejected';

export interface WorkflowPromptDraft {
  id: WorkflowPromptDraftId;
  workflowId: WorkflowId;
  runRequestId: WorkflowRunRequestId;
  seriesIndex: number;
  title: string;
  prompt: string;
  status: WorkflowPromptDraftStatus;
  createdAt: number;
  reviewedAt: number | null;
  reviewNote: string | null;
}

export type WorkflowRunStatus =
  | 'requested'
  | 'awaiting-review'
  | 'queued'
  | 'running'
  | 'succeeded'
  | 'failed'
  | 'cancelled';

export interface WorkflowRunFailure {
  code: string;
  message: string;
}

/**
 * Durable execution projection. Generated artifacts and detailed task history
 * stay in their owning NomiFun stores; this domain keeps UUIDv7 references.
 */
export interface WorkflowRunRecord {
  requestId: WorkflowRunRequestId;
  workflowId: WorkflowId;
  status: WorkflowRunStatus;
  promptDraftIds: WorkflowPromptDraftId[];
  taskIds: WorkflowTaskId[];
  resultAssetIds: WorkflowAssetId[];
  historyReferenceIds: WorkflowHistoryReferenceId[];
  queuedAt: number | null;
  startedAt: number | null;
  completedAt: number | null;
  failure: WorkflowRunFailure | null;
}

export interface WorkflowWorkspaceDocumentV1 {
  kind: 'nomifun.creative-studio.workflows';
  version: 1;
  workflows: WorkflowDefinitionV1[];
  promptDrafts: WorkflowPromptDraft[];
  runRequests: WorkflowRunRequest[];
  runs: WorkflowRunRecord[];
}

export type WorkflowValidationErrorCode =
  | 'invalid-json'
  | 'invalid-envelope'
  | 'unsupported-version'
  | 'invalid-value'
  | 'unknown-field'
  | 'limit-exceeded'
  | 'duplicate-id'
  | 'broken-reference'
  | 'cycle-detected'
  | 'invalid-transition';

export interface WorkflowValidationError {
  code: WorkflowValidationErrorCode;
  path: string;
  message: string;
}

export type WorkflowValidationResult =
  | { ok: true }
  | { ok: false; error: WorkflowValidationError };

export type WorkflowParseResult =
  | { ok: true; document: WorkflowWorkspaceDocumentV1 }
  | { ok: false; error: WorkflowValidationError };

export type WorkflowExportResult =
  | { ok: true; json: string; document: WorkflowWorkspaceDocumentV1 }
  | { ok: false; error: WorkflowValidationError };

export type WorkflowValueResult<Value> =
  | { ok: true; value: Value }
  | { ok: false; error: WorkflowValidationError };
