/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { WorkflowCommand } from './commands';
import { cloneWorkflowDefinition } from './model';
import type {
  WorkflowDefinitionV1,
  WorkflowPromptDraft,
  WorkflowRunRecord,
  WorkflowWorkspaceDocumentV1,
} from './types';
import {
  cloneWorkflowOutput,
  isWorkflowBusinessId,
  isWorkflowTerminalStatus,
  validateWorkflowDefinition,
  validateWorkflowInputsForDefinition,
  validateWorkflowWorkspaceDocument,
} from './validation';

const CODE = /^[a-z][a-z0-9._-]{0,79}$/;

function validTime(value: number): boolean {
  return Number.isSafeInteger(value) && value >= 0;
}

function validateCandidate(
  current: WorkflowWorkspaceDocumentV1,
  candidate: WorkflowWorkspaceDocumentV1
): WorkflowWorkspaceDocumentV1 {
  const result = validateWorkflowWorkspaceDocument(candidate);
  return result.ok ? candidate : current;
}

function updateDefinition(
  state: WorkflowWorkspaceDocumentV1,
  workflowId: string,
  updatedAt: number,
  mutate: (workflow: WorkflowDefinitionV1) => WorkflowDefinitionV1
): WorkflowWorkspaceDocumentV1 {
  const index = state.workflows.findIndex((workflow) => workflow.id === workflowId);
  if (index < 0 || !validTime(updatedAt)) return state;
  const current = state.workflows[index];
  if (updatedAt < current.metadata.updatedAt) return state;
  const changed = mutate(cloneWorkflowDefinition(current));
  if (JSON.stringify(changed) === JSON.stringify(current)) return state;
  const candidate = {
    ...changed,
    revision: current.revision + 1,
    metadata: { ...changed.metadata, updatedAt },
  };
  if (!validateWorkflowDefinition(candidate).ok) return state;
  const workflows = [...state.workflows];
  workflows[index] = candidate;
  return validateCandidate(state, { ...state, workflows });
}

function replaceById<Value extends { id: string }>(values: Value[], value: Value): Value[] {
  const index = values.findIndex((item) => item.id === value.id);
  if (index < 0) return [...values, value];
  const next = [...values];
  next[index] = value;
  return next;
}

function findRun(state: WorkflowWorkspaceDocumentV1, requestId: string) {
  const index = state.runs.findIndex((run) => run.requestId === requestId);
  return index < 0 ? null : { index, run: state.runs[index] };
}

function replaceRun(
  state: WorkflowWorkspaceDocumentV1,
  index: number,
  run: WorkflowRunRecord
): WorkflowWorkspaceDocumentV1 {
  const runs = [...state.runs];
  runs[index] = run;
  return validateCandidate(state, { ...state, runs });
}

export function workflowReducer(
  state: WorkflowWorkspaceDocumentV1,
  command: WorkflowCommand
): WorkflowWorkspaceDocumentV1 {
  switch (command.type) {
    case 'workflow/create': {
      if (!validateWorkflowDefinition(command.workflow).ok || state.workflows.some((item) => item.id === command.workflow.id)) return state;
      return validateCandidate(state, {
        ...state,
        workflows: [...state.workflows, cloneWorkflowDefinition(command.workflow)],
      });
    }
    case 'workflow/update-metadata':
      return updateDefinition(state, command.workflowId, command.updatedAt, (workflow) => ({
        ...workflow,
        metadata: {
          ...workflow.metadata,
          ...command.patch,
          tags: command.patch.tags ? [...command.patch.tags] : workflow.metadata.tags,
        },
      }));
    case 'workflow/delete': {
      const workflow = state.workflows.find((item) => item.id === command.workflowId);
      if (!workflow) return state;
      const hasActiveRun = state.runs.some(
        (run) => run.workflowId === workflow.id && !isWorkflowTerminalStatus(run.status)
      );
      if (hasActiveRun) return state;
      return validateCandidate(state, {
        ...state,
        workflows: state.workflows.filter((item) => item.id !== workflow.id),
      });
    }
    case 'variable/upsert':
      return updateDefinition(state, command.workflowId, command.updatedAt, (workflow) => ({
        ...workflow,
        variables: replaceById(workflow.variables, command.variable),
      }));
    case 'variable/delete':
      return updateDefinition(state, command.workflowId, command.updatedAt, (workflow) => {
        const referenced =
          workflow.templates.some((template) =>
            template.segments.some(
              (segment) => segment.kind === 'variable' && segment.variableId === command.variableId
            )
          ) ||
          workflow.steps.some(
            (step) =>
              step.kind === 'generate-images' &&
              step.referenceVariableIds.includes(command.variableId)
          );
        return referenced
          ? workflow
          : { ...workflow, variables: workflow.variables.filter((item) => item.id !== command.variableId) };
      });
    case 'template/upsert':
      return updateDefinition(state, command.workflowId, command.updatedAt, (workflow) => ({
        ...workflow,
        templates: replaceById(workflow.templates, command.template),
      }));
    case 'template/delete':
      return updateDefinition(state, command.workflowId, command.updatedAt, (workflow) => {
        const referenced = workflow.steps.some(
          (step) =>
            ((step.kind === 'render-template' || step.kind === 'draft-prompts') &&
              step.templateId === command.templateId) ||
            (step.kind === 'generate-images' &&
              step.promptSource.kind === 'template' &&
              step.promptSource.templateId === command.templateId)
        );
        return referenced
          ? workflow
          : { ...workflow, templates: workflow.templates.filter((item) => item.id !== command.templateId) };
      });
    case 'step/upsert':
      return updateDefinition(state, command.workflowId, command.updatedAt, (workflow) => ({
        ...workflow,
        steps: replaceById(workflow.steps, command.step),
      }));
    case 'step/delete':
      return updateDefinition(state, command.workflowId, command.updatedAt, (workflow) => {
        const referenced = workflow.steps.some(
          (step) =>
            step.dependsOn.includes(command.stepId) ||
            (step.kind === 'generate-images' &&
              step.promptSource.kind === 'prompt-drafts' &&
              step.promptSource.stepId === command.stepId) ||
            (step.kind === 'record-history' && step.sourceStepIds.includes(command.stepId))
        );
        return referenced
          ? workflow
          : { ...workflow, steps: workflow.steps.filter((item) => item.id !== command.stepId) };
      });
    case 'run/request': {
      const workflow = state.workflows.find((item) => item.id === command.workflowId);
      if (
        !workflow ||
        !isWorkflowBusinessId(command.id) ||
        !isWorkflowBusinessId(command.idempotencyKey) ||
        !validTime(command.requestedAt) ||
        !validateWorkflowInputsForDefinition(workflow, command.inputs).ok
      ) return state;
      const request = {
        id: command.id,
        idempotencyKey: command.idempotencyKey,
        workflowId: workflow.id,
        workflowRevision: workflow.revision,
        requestedAt: command.requestedAt,
        output: cloneWorkflowOutput(workflow.output),
        inputs: command.inputs.map((input) =>
          input.type === 'image-series' ? { ...input, assetIds: [...input.assetIds] } : { ...input }
        ),
      };
      const existing = state.runRequests.find(
        (item) => item.id === request.id || item.idempotencyKey === request.idempotencyKey
      );
      if (existing) return state;
      const run: WorkflowRunRecord = {
        requestId: request.id,
        workflowId: request.workflowId,
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
        !isWorkflowBusinessId(command.id) ||
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
      const draft: WorkflowPromptDraft = {
        id: command.id,
        workflowId: request.workflowId,
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
      if (!located || located.run.status !== 'queued' || !validTime(command.startedAt) || command.startedAt < (located.run.queuedAt ?? 0) || command.taskIds.length === 0 || new Set(command.taskIds).size !== command.taskIds.length || command.taskIds.some((id) => !isWorkflowBusinessId(id))) return state;
      return replaceRun(state, located.index, {
        ...located.run,
        status: 'running',
        taskIds: [...command.taskIds],
        startedAt: command.startedAt,
      });
    }
    case 'run/succeed': {
      const located = findRun(state, command.requestId);
      if (!located || located.run.status !== 'running' || !validTime(command.completedAt) || command.completedAt < (located.run.startedAt ?? 0) || command.resultAssetIds.length === 0 || [...command.resultAssetIds, ...command.historyReferenceIds].some((id) => !isWorkflowBusinessId(id)) || new Set(command.resultAssetIds).size !== command.resultAssetIds.length || new Set(command.historyReferenceIds).size !== command.historyReferenceIds.length) return state;
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
      if (!located || !request || isWorkflowTerminalStatus(located.run.status) || !validTime(command.completedAt) || command.completedAt < request.requestedAt || (located.run.startedAt !== null && command.completedAt < located.run.startedAt)) return state;
      return replaceRun(state, located.index, {
        ...located.run,
        status: 'cancelled',
        completedAt: command.completedAt,
      });
    }
  }
}
