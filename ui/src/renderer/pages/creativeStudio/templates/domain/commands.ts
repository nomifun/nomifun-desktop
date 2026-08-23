/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  CreativeTemplateDefinitionV1,
  CreativeTemplateInputValue,
  CreativeTemplateMetadata,
  CreativeTemplateStep,
  CreativePromptTemplate,
  CreativeTemplateVariable,
} from './types';

export type CreativeTemplateMetadataPatch = Partial<
  Pick<CreativeTemplateMetadata, 'name' | 'description' | 'category' | 'visibility' | 'tags'>
>;

export type CreativeTemplateCommand =
  | { type: 'template/create'; template: CreativeTemplateDefinitionV1 }
  | { type: 'template/update-metadata'; templateId: string; patch: CreativeTemplateMetadataPatch; updatedAt: number }
  | { type: 'template/delete'; templateId: string }
  | { type: 'variable/upsert'; templateId: string; variable: CreativeTemplateVariable; updatedAt: number }
  | { type: 'variable/delete'; templateId: string; variableId: string; updatedAt: number }
  | {
      type: 'prompt-template/upsert';
      templateId: string;
      promptTemplate: CreativePromptTemplate;
      updatedAt: number;
    }
  | {
      type: 'prompt-template/delete';
      templateId: string;
      promptTemplateId: string;
      updatedAt: number;
    }
  | { type: 'step/upsert'; templateId: string; step: CreativeTemplateStep; updatedAt: number }
  | { type: 'step/delete'; templateId: string; stepId: string; updatedAt: number }
  | {
      type: 'run/request';
      id: string;
      idempotencyKey: string;
      templateId: string;
      requestedAt: number;
      inputs: CreativeTemplateInputValue[];
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

export const templateCommands = {
  create: (template: CreativeTemplateDefinitionV1): CreativeTemplateCommand => ({ type: 'template/create', template }),
  delete: (templateId: string): CreativeTemplateCommand => ({ type: 'template/delete', templateId }),
  requestRun: (input: Omit<Extract<CreativeTemplateCommand, { type: 'run/request' }>, 'type'>): CreativeTemplateCommand => ({ type: 'run/request', ...input }),
  queueRun: (requestId: string, queuedAt: number): CreativeTemplateCommand => ({ type: 'run/queue', requestId, queuedAt }),
} as const;
