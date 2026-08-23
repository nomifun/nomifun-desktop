/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { CreativeStudioContractError } from '../../domain';
import type { CreativeModelSelectionRef } from '../../models';
import {
  validateWorkflowDefinition,
  type WorkflowDefinitionV1,
  type WorkflowTemplateSegment,
} from '../domain';
import {
  createWorkflowTranslationCopy,
  type WorkflowTranslationCopy,
} from '../workflowI18n';
import { createBlankWorkflow } from '../page/workflowViewModel';

import type { CreativeWorkflowDraftArtifact } from './artifacts';

const WORKFLOW_VARIABLE_KEY = /^[a-z][a-z0-9_]{0,63}$/u;

const fail = (path: string, expected: string): never => {
  throw new CreativeStudioContractError('INVALID_RESPONSE', path, expected);
};

const parsePromptTemplate = (
  text: string,
  workflow: WorkflowDefinitionV1
): WorkflowTemplateSegment[] => {
  const variables = new Map(workflow.variables.map((variable) => [variable.key, variable.id]));
  const segments: WorkflowTemplateSegment[] = [];
  let cursor = 0;

  while (cursor < text.length) {
    const opening = text.indexOf('{{', cursor);
    const unmatchedClosing = text.indexOf('}}', cursor);
    if (opening < 0) {
      if (unmatchedClosing >= 0) {
        return fail('$.draft.promptTemplate', 'placeholder opening before every }}');
      }
      segments.push({ kind: 'text', text: text.slice(cursor) });
      break;
    }
    if (unmatchedClosing >= 0 && unmatchedClosing < opening) {
      return fail('$.draft.promptTemplate', 'placeholder opening before every }}');
    }
    if (opening > cursor) {
      segments.push({ kind: 'text', text: text.slice(cursor, opening) });
    }

    const closing = text.indexOf('}}', opening + 2);
    if (closing < 0) {
      return fail('$.draft.promptTemplate', 'closed {{variable_key}} placeholder');
    }
    const expression = text.slice(opening + 2, closing);
    if (expression.includes('{{')) {
      return fail('$.draft.promptTemplate', 'non-nested {{variable_key}} placeholder');
    }
    const key = expression.trim();
    if (!WORKFLOW_VARIABLE_KEY.test(key)) {
      return fail('$.draft.promptTemplate', 'canonical {{variable_key}} placeholder');
    }
    const variableId = variables.get(key);
    if (!variableId) {
      return fail(
        '$.draft.promptTemplate',
        `only the fixed variables ${[...variables.keys()].join(', ')}`
      );
    }
    segments.push({ kind: 'variable', variableId });
    cursor = closing + 2;
  }

  if (!segments.some((segment) => segment.kind === 'variable')) {
    return fail('$.draft.promptTemplate', 'at least one allowed variable placeholder');
  }
  return segments;
};

/**
 * Convert one already-parsed Agent artifact into the existing workflow domain.
 * The model never owns IDs, revisions, timestamps, visibility, tags or model
 * bindings: all of those originate from the product template and current turn.
 */
export function convertCreativeWorkflowDraft(
  artifact: CreativeWorkflowDraftArtifact,
  exactChatModel: CreativeModelSelectionRef,
  copy: WorkflowTranslationCopy = createWorkflowTranslationCopy()
): WorkflowDefinitionV1 {
  const workflow = createBlankWorkflow(artifact.draft.mode, copy);
  workflow.metadata = {
    name: artifact.draft.name,
    description: artifact.draft.description,
    category: artifact.draft.category,
    visibility: 'private',
    tags: [],
    createdAt: workflow.metadata.createdAt,
    updatedAt: workflow.metadata.updatedAt,
  };

  const template = workflow.templates[0];
  if (!template) return fail('$.templates', 'blank workflow prompt template');
  template.segments = parsePromptTemplate(artifact.draft.promptTemplate, workflow);

  for (const step of workflow.steps) {
    if (step.kind === 'generate-images') step.generation.model = null;
    if (step.kind === 'draft-prompts') {
      step.planning.model = {
        providerId: exactChatModel.providerId,
        model: exactChatModel.model,
        task: 'chat',
      };
    }
  }

  const validation = validateWorkflowDefinition(workflow);
  if (!validation.ok) {
    return fail(
      validation.error.path,
      `valid workflow definition (${validation.error.message})`
    );
  }
  return workflow;
}
