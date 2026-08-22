/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  WorkflowDefinitionV1,
  WorkflowInputValue,
  WorkflowMetadata,
  WorkflowStep,
  WorkflowTemplate,
  WorkflowVariable,
} from './types';

export type WorkflowMetadataPatch = Partial<
  Pick<WorkflowMetadata, 'name' | 'description' | 'category' | 'visibility' | 'tags'>
>;

export type WorkflowCommand =
  | { type: 'workflow/create'; workflow: WorkflowDefinitionV1 }
  | { type: 'workflow/update-metadata'; workflowId: string; patch: WorkflowMetadataPatch; updatedAt: number }
  | { type: 'workflow/delete'; workflowId: string }
  | { type: 'variable/upsert'; workflowId: string; variable: WorkflowVariable; updatedAt: number }
  | { type: 'variable/delete'; workflowId: string; variableId: string; updatedAt: number }
  | { type: 'template/upsert'; workflowId: string; template: WorkflowTemplate; updatedAt: number }
  | { type: 'template/delete'; workflowId: string; templateId: string; updatedAt: number }
  | { type: 'step/upsert'; workflowId: string; step: WorkflowStep; updatedAt: number }
  | { type: 'step/delete'; workflowId: string; stepId: string; updatedAt: number }
  | {
      type: 'run/request';
      id: string;
      idempotencyKey: string;
      workflowId: string;
      requestedAt: number;
      inputs: WorkflowInputValue[];
      referenceAssetIds: string[];
    }
  | {
      type: 'prompt-draft/add';
      id: string;
      runRequestId: string;
      seriesIndex: number;
      title: string;
      prompt: string;
      createdAt: number;
    }
  | { type: 'prompt-draft/edit'; draftId: string; title: string; prompt: string }
  | { type: 'prompt-draft/approve'; draftId: string; reviewedAt: number; note: string | null }
  | { type: 'prompt-draft/reject'; draftId: string; reviewedAt: number; note: string }
  | { type: 'prompt-draft/delete'; draftId: string }
  | { type: 'run/queue'; requestId: string; queuedAt: number }
  | { type: 'run/start'; requestId: string; taskIds: string[]; startedAt: number }
  | {
      type: 'run/succeed';
      requestId: string;
      resultAssetIds: string[];
      historyReferenceIds: string[];
      completedAt: number;
    }
  | { type: 'run/fail'; requestId: string; code: string; message: string; completedAt: number }
  | { type: 'run/cancel'; requestId: string; completedAt: number };

export const workflowCommands = {
  create: (workflow: WorkflowDefinitionV1): WorkflowCommand => ({ type: 'workflow/create', workflow }),
  delete: (workflowId: string): WorkflowCommand => ({ type: 'workflow/delete', workflowId }),
  requestRun: (input: Omit<Extract<WorkflowCommand, { type: 'run/request' }>, 'type'>): WorkflowCommand => ({ type: 'run/request', ...input }),
  queueRun: (requestId: string, queuedAt: number): WorkflowCommand => ({ type: 'run/queue', requestId, queuedAt }),
} as const;
