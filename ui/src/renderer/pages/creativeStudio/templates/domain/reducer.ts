/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeTemplateCommand } from './commands';
import { cloneTemplateDefinition } from './model';
import type {
  CreativeTemplateDefinitionV1,
  CreativeTemplatePromptDraft,
  CreativeTemplateRunRecord,
  CreativeTemplateWorkspaceDocumentV1,
} from './types';
import {
  cloneTemplateOutput,
  isTemplateBusinessId,
  isTemplateTerminalStatus,
  validateTemplateDefinition,
  validateTemplateInputsForDefinition,
  validateTemplateWorkspaceDocument,
} from './validation';

const CODE = /^[a-z][a-z0-9._-]{0,79}$/;

function validTime(value: number): boolean {
  return Number.isSafeInteger(value) && value >= 0;
}

function validateCandidate(
  current: CreativeTemplateWorkspaceDocumentV1,
  candidate: CreativeTemplateWorkspaceDocumentV1
): CreativeTemplateWorkspaceDocumentV1 {
  const result = validateTemplateWorkspaceDocument(candidate);
  return result.ok ? candidate : current;
}

function updateDefinition(
  state: CreativeTemplateWorkspaceDocumentV1,
  templateId: string,
  updatedAt: number,
  mutate: (template: CreativeTemplateDefinitionV1) => CreativeTemplateDefinitionV1
): CreativeTemplateWorkspaceDocumentV1 {
  const index = state.templates.findIndex((template) => template.id === templateId);
  if (index < 0 || !validTime(updatedAt)) return state;
  const current = state.templates[index];
  if (updatedAt < current.metadata.updatedAt) return state;
  const changed = mutate(cloneTemplateDefinition(current));
  if (JSON.stringify(changed) === JSON.stringify(current)) return state;
  const candidate = {
    ...changed,
    revision: current.revision + 1,
    metadata: { ...changed.metadata, updatedAt },
  };
  if (!validateTemplateDefinition(candidate).ok) return state;
  const templates = [...state.templates];
  templates[index] = candidate;
  return validateCandidate(state, { ...state, templates });
}

function replaceById<Value extends { id: string }>(values: Value[], value: Value): Value[] {
  const index = values.findIndex((item) => item.id === value.id);
  if (index < 0) return [...values, value];
  const next = [...values];
  next[index] = value;
  return next;
}

function findRun(state: CreativeTemplateWorkspaceDocumentV1, requestId: string) {
  const index = state.runs.findIndex((run) => run.requestId === requestId);
  return index < 0 ? null : { index, run: state.runs[index] };
}

function replaceRun(
  state: CreativeTemplateWorkspaceDocumentV1,
  index: number,
  run: CreativeTemplateRunRecord
): CreativeTemplateWorkspaceDocumentV1 {
  const runs = [...state.runs];
  runs[index] = run;
  return validateCandidate(state, { ...state, runs });
}

export function templateReducer(
  state: CreativeTemplateWorkspaceDocumentV1,
  command: CreativeTemplateCommand
): CreativeTemplateWorkspaceDocumentV1 {
  switch (command.type) {
    case 'template/create': {
      if (!validateTemplateDefinition(command.template).ok || state.templates.some((item) => item.id === command.template.id)) return state;
      return validateCandidate(state, {
        ...state,
        templates: [...state.templates, cloneTemplateDefinition(command.template)],
      });
    }
    case 'template/update-metadata':
      return updateDefinition(state, command.templateId, command.updatedAt, (template) => ({
        ...template,
        metadata: {
          ...template.metadata,
          ...command.patch,
          tags: command.patch.tags ? [...command.patch.tags] : template.metadata.tags,
        },
      }));
    case 'template/delete': {
      const template = state.templates.find((item) => item.id === command.templateId);
      if (!template) return state;
      const hasActiveRun = state.runs.some(
        (run) => run.templateId === template.id && !isTemplateTerminalStatus(run.status)
      );
      if (hasActiveRun) return state;
      return validateCandidate(state, {
        ...state,
        templates: state.templates.filter((item) => item.id !== template.id),
      });
    }
    case 'variable/upsert':
      return updateDefinition(state, command.templateId, command.updatedAt, (template) => ({
        ...template,
        variables: replaceById(template.variables, command.variable),
      }));
    case 'variable/delete':
      return updateDefinition(state, command.templateId, command.updatedAt, (template) => {
        const referenced =
          template.templates.some((promptTemplate) =>
            promptTemplate.segments.some(
              (segment) => segment.kind === 'variable' && segment.variableId === command.variableId
            )
          ) ||
          template.steps.some(
            (step) =>
              step.kind === 'generate-images' &&
              step.referenceVariableIds.includes(command.variableId)
          );
        return referenced
          ? template
          : { ...template, variables: template.variables.filter((item) => item.id !== command.variableId) };
      });
    case 'prompt-template/upsert':
      return updateDefinition(state, command.templateId, command.updatedAt, (template) => ({
        ...template,
        templates: replaceById(template.templates, command.promptTemplate),
      }));
    case 'prompt-template/delete':
      return updateDefinition(state, command.templateId, command.updatedAt, (template) => {
        const referenced = template.steps.some(
          (step) =>
            ((step.kind === 'render-template' || step.kind === 'draft-prompts') &&
              step.templateId === command.promptTemplateId) ||
            (step.kind === 'generate-images' &&
              step.promptSource.kind === 'template' &&
              step.promptSource.templateId === command.promptTemplateId)
        );
        return referenced
          ? template
          : {
              ...template,
              templates: template.templates.filter(
                (item) => item.id !== command.promptTemplateId
              ),
            };
      });
    case 'step/upsert':
      return updateDefinition(state, command.templateId, command.updatedAt, (template) => ({
        ...template,
        steps: replaceById(template.steps, command.step),
      }));
    case 'step/delete':
      return updateDefinition(state, command.templateId, command.updatedAt, (template) => {
        const referenced = template.steps.some(
          (step) =>
            step.dependsOn.includes(command.stepId) ||
            (step.kind === 'generate-images' &&
              step.promptSource.kind === 'prompt-drafts' &&
              step.promptSource.stepId === command.stepId) ||
            (step.kind === 'record-history' && step.sourceStepIds.includes(command.stepId))
        );
        return referenced
          ? template
          : { ...template, steps: template.steps.filter((item) => item.id !== command.stepId) };
      });
    case 'run/request': {
      const template = state.templates.find((item) => item.id === command.templateId);
      if (
        !template ||
        !isTemplateBusinessId(command.id) ||
        !isTemplateBusinessId(command.idempotencyKey) ||
        command.idempotencyKey !== command.id ||
        !validTime(command.requestedAt) ||
        !validateTemplateInputsForDefinition(template, command.inputs).ok ||
        command.referenceAssetIds.some((assetId) => !isTemplateBusinessId(assetId)) ||
        new Set(command.referenceAssetIds).size !== command.referenceAssetIds.length ||
        command.referenceAssetIds.length > 100
      ) return state;
      const request = {
        id: command.id,
        idempotencyKey: command.idempotencyKey,
        templateId: template.id,
        templateRevision: template.revision,
        requestedAt: command.requestedAt,
        output: cloneTemplateOutput(template.output),
        inputs: command.inputs.map((input) =>
          input.type === 'image-series' ? { ...input, assetIds: [...input.assetIds] } : { ...input }
        ),
        referenceAssetIds: [...command.referenceAssetIds],
      };
      const existing = state.runRequests.find(
        (item) => item.id === request.id || item.idempotencyKey === request.idempotencyKey
      );
      if (existing) return state;
      const run: CreativeTemplateRunRecord = {
        requestId: request.id,
        templateId: request.templateId,
        status: request.output.kind === 'multi-image-series' ? 'awaiting-review' : 'requested',
        promptDraftIds: [],
        taskIds: [],
        resultAssetIds: [],
        historyReferenceIds: [],
        queuedAt: null,
        startedAt: null,
        completedAt: null,
        failure: null,
      };
      return validateCandidate(state, {
        ...state,
        runRequests: [...state.runRequests, request],
        runs: [...state.runs, run],
      });
    }
    case 'prompt-draft/add': {
      const located = findRun(state, command.runRequestId);
      const request = state.runRequests.find((item) => item.id === command.runRequestId);
      if (
        !located ||
        !request ||
        located.run.status !== 'awaiting-review' ||
        request.output.kind !== 'multi-image-series' ||
        !isTemplateBusinessId(command.id) ||
        !validTime(command.createdAt) ||
        command.createdAt < request.requestedAt ||
        !Number.isSafeInteger(command.seriesIndex) ||
        command.seriesIndex < 0 ||
        command.seriesIndex >= request.output.targetCount ||
        !command.title.trim() ||
        !command.prompt.trim() ||
        state.promptDrafts.some(
          (draft) =>
            draft.id === command.id ||
            (draft.runRequestId === request.id && draft.seriesIndex === command.seriesIndex)
        )
      ) return state;
      const automaticallyApproved = !request.output.reviewRequired;
      const draft: CreativeTemplatePromptDraft = {
        id: command.id,
        templateId: request.templateId,
        runRequestId: request.id,
        seriesIndex: command.seriesIndex,
        title: command.title.trim(),
        prompt: command.prompt.trim(),
        status: automaticallyApproved ? 'approved' : 'pending-review',
        createdAt: command.createdAt,
        reviewedAt: automaticallyApproved ? command.createdAt : null,
        reviewNote: null,
      };
      const runs = [...state.runs];
      runs[located.index] = {
        ...located.run,
        promptDraftIds: [...located.run.promptDraftIds, draft.id],
      };
      return validateCandidate(state, {
        ...state,
        promptDrafts: [...state.promptDrafts, draft],
        runs,
      });
    }
    case 'prompt-draft/edit': {
      const index = state.promptDrafts.findIndex((draft) => draft.id === command.draftId);
      if (index < 0 || !command.title.trim() || !command.prompt.trim()) return state;
      const draft = state.promptDrafts[index];
      const run = state.runs.find((item) => item.requestId === draft.runRequestId);
      const request = state.runRequests.find((item) => item.id === draft.runRequestId);
      if (!run || run.status !== 'awaiting-review' || !request || request.output.kind !== 'multi-image-series') return state;
      const automatic = !request.output.reviewRequired;
      const promptDrafts = [...state.promptDrafts];
      promptDrafts[index] = {
        ...draft,
        title: command.title.trim(),
        prompt: command.prompt.trim(),
        status: automatic ? 'approved' : 'pending-review',
        reviewedAt: automatic ? draft.createdAt : null,
        reviewNote: null,
      };
      return validateCandidate(state, { ...state, promptDrafts });
    }
    case 'prompt-draft/approve':
    case 'prompt-draft/reject': {
      const index = state.promptDrafts.findIndex((draft) => draft.id === command.draftId);
      if (index < 0 || !validTime(command.reviewedAt)) return state;
      const draft = state.promptDrafts[index];
      const run = state.runs.find((item) => item.requestId === draft.runRequestId);
      const request = state.runRequests.find((item) => item.id === draft.runRequestId);
      if (!run || run.status !== 'awaiting-review' || !request || request.output.kind !== 'multi-image-series' || !request.output.reviewRequired || command.reviewedAt < draft.createdAt) return state;
      if (command.type === 'prompt-draft/reject' && !command.note.trim()) return state;
      const promptDrafts = [...state.promptDrafts];
      promptDrafts[index] = {
        ...draft,
        status: command.type === 'prompt-draft/approve' ? 'approved' : 'rejected',
        reviewedAt: command.reviewedAt,
        reviewNote: command.note,
      };
      return validateCandidate(state, { ...state, promptDrafts });
    }
    case 'prompt-draft/delete': {
      const draft = state.promptDrafts.find((item) => item.id === command.draftId);
      if (!draft) return state;
      const located = findRun(state, draft.runRequestId);
      if (!located || located.run.status !== 'awaiting-review') return state;
      const runs = [...state.runs];
      runs[located.index] = {
        ...located.run,
        promptDraftIds: located.run.promptDraftIds.filter((id) => id !== draft.id),
      };
      return validateCandidate(state, {
        ...state,
        promptDrafts: state.promptDrafts.filter((item) => item.id !== draft.id),
        runs,
      });
    }
    case 'run/queue': {
      const located = findRun(state, command.requestId);
      const request = state.runRequests.find((item) => item.id === command.requestId);
      if (!located || !request || !validTime(command.queuedAt) || command.queuedAt < request.requestedAt || (located.run.status !== 'requested' && located.run.status !== 'awaiting-review')) return state;
      if (request.output.kind === 'multi-image-series') {
        const drafts = state.promptDrafts.filter((draft) => draft.runRequestId === request.id);
        if (drafts.length !== request.output.targetCount || (request.output.reviewRequired && drafts.some((draft) => draft.status !== 'approved'))) return state;
      }
      return replaceRun(state, located.index, {
        ...located.run,
        status: 'queued',
        queuedAt: command.queuedAt,
      });
    }
    case 'run/start': {
      const located = findRun(state, command.requestId);
      if (!located || located.run.status !== 'queued' || !validTime(command.startedAt) || command.startedAt < (located.run.queuedAt ?? 0) || command.taskIds.length === 0 || new Set(command.taskIds).size !== command.taskIds.length || command.taskIds.some((id) => !isTemplateBusinessId(id))) return state;
      return replaceRun(state, located.index, {
        ...located.run,
        status: 'running',
        taskIds: [...command.taskIds],
        startedAt: command.startedAt,
      });
    }
    case 'run/succeed': {
      const located = findRun(state, command.requestId);
      if (!located || located.run.status !== 'running' || !validTime(command.completedAt) || command.completedAt < (located.run.startedAt ?? 0) || command.resultAssetIds.length === 0 || [...command.resultAssetIds, ...command.historyReferenceIds].some((id) => !isTemplateBusinessId(id)) || new Set(command.resultAssetIds).size !== command.resultAssetIds.length || new Set(command.historyReferenceIds).size !== command.historyReferenceIds.length) return state;
      return replaceRun(state, located.index, {
        ...located.run,
        status: 'succeeded',
        resultAssetIds: [...command.resultAssetIds],
        historyReferenceIds: [...command.historyReferenceIds],
        completedAt: command.completedAt,
      });
    }
    case 'run/fail': {
      const located = findRun(state, command.requestId);
      const request = state.runRequests.find((item) => item.id === command.requestId);
      if (!located || !request || (located.run.status !== 'queued' && located.run.status !== 'running') || !validTime(command.completedAt) || command.completedAt < request.requestedAt || (located.run.startedAt !== null && command.completedAt < located.run.startedAt) || !CODE.test(command.code) || !command.message.trim()) return state;
      return replaceRun(state, located.index, {
        ...located.run,
        status: 'failed',
        completedAt: command.completedAt,
        failure: { code: command.code, message: command.message.trim() },
      });
    }
    case 'run/cancel': {
      const located = findRun(state, command.requestId);
      const request = state.runRequests.find((item) => item.id === command.requestId);
      if (!located || !request || isTemplateTerminalStatus(located.run.status) || !validTime(command.completedAt) || command.completedAt < request.requestedAt || (located.run.startedAt !== null && command.completedAt < located.run.startedAt)) return state;
      return replaceRun(state, located.index, {
        ...located.run,
        status: 'cancelled',
        completedAt: command.completedAt,
      });
    }
  }
}
