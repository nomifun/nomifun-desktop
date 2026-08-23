/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

export type CreativeStudioTemplateId = string;
export type CreativeTemplateVariableId = string;
export type CreativePromptTemplateId = string;
export type CreativeStudioTemplateStepId = string;
export type CreativeStudioTemplateRunId = string;
export type CreativeTemplatePromptDraftId = string;
export type CreativeTemplateAssetId = string;
export type CreativeTemplateTaskId = string;
export type CreativeTemplateHistoryReferenceId = string;

export type CreativeTemplateVisibility = 'private' | 'public';

/** Typed, user-facing catalog information. There is no opaque metadata bag. */
export interface CreativeTemplateMetadata {
  name: string;
  description: string;
  category: string;
  visibility: CreativeTemplateVisibility;
  tags: string[];
  createdAt: number;
  updatedAt: number;
}

interface CreativeTemplateVariableBase {
  id: CreativeTemplateVariableId;
  key: string;
  label: string;
  description: string;
  required: boolean;
}

export interface CreativeTemplateTextVariable extends CreativeTemplateVariableBase {
  type: 'text' | 'multiline-text';
  defaultValue: string | null;
  placeholder: string;
  minLength: number;
  maxLength: number;
}

export interface CreativeTemplateNumberVariable extends CreativeTemplateVariableBase {
  type: 'number';
  defaultValue: number | null;
  minimum: number | null;
  maximum: number | null;
  step: number | null;
}

export interface CreativeTemplateBooleanVariable extends CreativeTemplateVariableBase {
  type: 'boolean';
  defaultValue: boolean;
}

export interface CreativeTemplateChoiceVariable extends CreativeTemplateVariableBase {
  type: 'choice';
  defaultValue: string | null;
  options: string[];
}

export interface CreativeTemplateImageVariable extends CreativeTemplateVariableBase {
  type: 'image';
  defaultAssetId: CreativeTemplateAssetId | null;
}

export interface CreativeTemplateImageSeriesVariable extends CreativeTemplateVariableBase {
  type: 'image-series';
  defaultAssetIds: CreativeTemplateAssetId[];
  minItems: number;
  maxItems: number;
}

export type CreativeTemplateVariable =
  | CreativeTemplateTextVariable
  | CreativeTemplateNumberVariable
  | CreativeTemplateBooleanVariable
  | CreativeTemplateChoiceVariable
  | CreativeTemplateImageVariable
  | CreativeTemplateImageSeriesVariable;

export type CreativePromptTemplateSegment =
  | { kind: 'text'; text: string }
  | { kind: 'variable'; variableId: CreativeTemplateVariableId };

export interface CreativePromptTemplate {
  id: CreativePromptTemplateId;
  name: string;
  segments: CreativePromptTemplateSegment[];
}

export type CreativeTemplateOutputPlan =
  | { kind: 'single-image' }
  | {
      kind: 'multi-image-series';
      targetCount: number;
      concurrency: number;
      reviewRequired: boolean;
    };

interface CreativeTemplateStepBase {
  id: CreativeStudioTemplateStepId;
  name: string;
  dependsOn: CreativeStudioTemplateStepId[];
  enabled: boolean;
}

export interface CreativeTemplateRenderPromptStep extends CreativeTemplateStepBase {
  kind: 'render-template';
  templateId: CreativePromptTemplateId;
}

export interface CreativeTemplateDraftPromptsStep extends CreativeTemplateStepBase {
  kind: 'draft-prompts';
  templateId: CreativePromptTemplateId;
  planning: CreativeTemplatePromptPlanningSettings;
}

export interface CreativeTemplateTextModelBinding {
  providerId: string;
  model: string;
  task: 'chat';
}

/** Exact NomiFun Chat model used to turn one rendered brief into reviewed prompts. */
export interface CreativeTemplatePromptPlanningSettings {
  model: CreativeTemplateTextModelBinding | null;
  instruction: string;
  maxTokens: number;
}

export type CreativeTemplatePromptSource =
  | { kind: 'template'; templateId: CreativePromptTemplateId }
  | { kind: 'prompt-drafts'; stepId: CreativeStudioTemplateStepId };

export type CreativeTemplateImageTask = 'image_generation' | 'image_edit';

export interface CreativeTemplateImageModelBinding {
  providerId: string;
  model: string;
  task: CreativeTemplateImageTask;
}

export type CreativeTemplateImageQuality = 'auto' | 'high' | 'medium' | 'low';

/** Provider-neutral settings persisted with the template definition. */
export interface CreativeTemplateImageGenerationSettings {
  model: CreativeTemplateImageModelBinding | null;
  quality: CreativeTemplateImageQuality;
  width: number;
  height: number;
  imagesPerPrompt: number;
}

export interface CreativeTemplateGenerateImagesStep extends CreativeTemplateStepBase {
  kind: 'generate-images';
  promptSource: CreativeTemplatePromptSource;
  referenceVariableIds: CreativeTemplateVariableId[];
  generation: CreativeTemplateImageGenerationSettings;
}

export interface CreativeTemplateRecordHistoryStep extends CreativeTemplateStepBase {
  kind: 'record-history';
  sourceStepIds: CreativeStudioTemplateStepId[];
}

export type CreativeTemplateStep =
  | CreativeTemplateRenderPromptStep
  | CreativeTemplateDraftPromptsStep
  | CreativeTemplateGenerateImagesStep
  | CreativeTemplateRecordHistoryStep;

export interface CreativeTemplateDefinitionV1 {
  id: CreativeStudioTemplateId;
  revision: number;
  metadata: CreativeTemplateMetadata;
  output: CreativeTemplateOutputPlan;
  variables: CreativeTemplateVariable[];
  templates: CreativePromptTemplate[];
  steps: CreativeTemplateStep[];
}

export type CreativeTemplateInputValue =
  | { variableId: CreativeTemplateVariableId; type: 'text' | 'multiline-text' | 'choice'; value: string }
  | { variableId: CreativeTemplateVariableId; type: 'number'; value: number }
  | { variableId: CreativeTemplateVariableId; type: 'boolean'; value: boolean }
  | { variableId: CreativeTemplateVariableId; type: 'image'; assetId: CreativeTemplateAssetId | null }
  | { variableId: CreativeTemplateVariableId; type: 'image-series'; assetIds: CreativeTemplateAssetId[] };

export interface CreativeTemplateRunRequest {
  id: CreativeStudioTemplateRunId;
  idempotencyKey: string;
  templateId: CreativeStudioTemplateId;
  templateRevision: number;
  requestedAt: number;
  output: CreativeTemplateOutputPlan;
  inputs: CreativeTemplateInputValue[];
  referenceAssetIds: CreativeTemplateAssetId[];
}

export type CreativeTemplatePromptDraftStatus = 'pending-review' | 'approved' | 'rejected';

export interface CreativeTemplatePromptDraft {
  id: CreativeTemplatePromptDraftId;
  templateId: CreativeStudioTemplateId;
  runRequestId: CreativeStudioTemplateRunId;
  seriesIndex: number;
  title: string;
  prompt: string;
  status: CreativeTemplatePromptDraftStatus;
  createdAt: number;
  reviewedAt: number | null;
  reviewNote: string | null;
}

export type CreativeTemplateRunStatus =
  | 'requested'
  | 'awaiting-review'
  | 'queued'
  | 'running'
  | 'succeeded'
  | 'failed'
  | 'cancelled';

export interface CreativeTemplateRunFailure {
  code: string;
  message: string;
}

/**
 * Durable execution projection. Generated artifacts and detailed task history
 * stay in their owning NomiFun stores; this domain keeps UUIDv7 references.
 */
export interface CreativeTemplateRunRecord {
  requestId: CreativeStudioTemplateRunId;
  templateId: CreativeStudioTemplateId;
  status: CreativeTemplateRunStatus;
  promptDraftIds: CreativeTemplatePromptDraftId[];
  taskIds: CreativeTemplateTaskId[];
  resultAssetIds: CreativeTemplateAssetId[];
  historyReferenceIds: CreativeTemplateHistoryReferenceId[];
  queuedAt: number | null;
  startedAt: number | null;
  completedAt: number | null;
  failure: CreativeTemplateRunFailure | null;
}

/**
 * Server-authoritative durable run. The definition and request are immutable
 * snapshots; only the revisioned execution projection may advance.
 */
export interface CreativeTemplateRunAggregateV1 {
  kind: 'nomifun.creative-studio.template-run';
  version: 1;
  revision: number;
  templateSnapshot: CreativeTemplateDefinitionV1;
  request: CreativeTemplateRunRequest;
  promptDrafts: CreativeTemplatePromptDraft[];
  record: CreativeTemplateRunRecord;
}

export interface CreativeTemplateWorkspaceDocumentV1 {
  kind: 'nomifun.creative-studio.templates';
  version: 1;
  templates: CreativeTemplateDefinitionV1[];
  promptDrafts: CreativeTemplatePromptDraft[];
  runRequests: CreativeTemplateRunRequest[];
  runs: CreativeTemplateRunRecord[];
}

export type CreativeTemplateValidationErrorCode =
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

export interface CreativeTemplateValidationError {
  code: CreativeTemplateValidationErrorCode;
  path: string;
  message: string;
}

export type CreativeTemplateValidationResult =
  | { ok: true }
  | { ok: false; error: CreativeTemplateValidationError };

export type CreativeTemplateParseResult =
  | { ok: true; document: CreativeTemplateWorkspaceDocumentV1 }
  | { ok: false; error: CreativeTemplateValidationError };

export type CreativeTemplateExportResult =
  | { ok: true; json: string; document: CreativeTemplateWorkspaceDocumentV1 }
  | { ok: false; error: CreativeTemplateValidationError };

export type CreativeTemplateValueResult<Value> =
  | { ok: true; value: Value }
  | { ok: false; error: CreativeTemplateValidationError };
