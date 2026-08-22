/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { httpRequest } from '@/common/adapter/httpBridge';
import { CANONICAL_UUID_V7, type ProviderId } from '@/common/types/ids';

import { MAX_CREATIVE_WORKFLOW_DRAFT_JSON_BYTES } from './artifacts';

export const CREATIVE_STUDIO_WORKFLOW_DRAFTS_ENDPOINT =
  '/api/creative-studio/workflow-drafts';

const MAX_PROMPT_CHARACTERS = 20_000;
const MAX_MODEL_CHARACTERS = 512;
const MAX_RESPONSE_UTF8_BYTES = MAX_CREATIVE_WORKFLOW_DRAFT_JSON_BYTES + 12;

export interface WorkflowDraftPortInput {
  providerId: ProviderId;
  model: string;
  prompt: string;
}

export interface WorkflowDraftPortResult {
  text: string;
}

export interface WorkflowDraftPort {
  draft(input: WorkflowDraftPortInput): Promise<WorkflowDraftPortResult>;
}

export type WorkflowDraftHttpRequest = (
  method: string,
  path: string,
  body: unknown
) => Promise<unknown>;

export class WorkflowDraftPortError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'WorkflowDraftPortError';
  }
}

const defaultRequest: WorkflowDraftHttpRequest = (method, path, body) =>
  httpRequest<unknown>(method, path, body, { timeoutMs: 125_000 });

const canonicalText = (
  value: string,
  label: string,
  maximum: number
): string => {
  if (!value || value !== value.trim() || value.length > maximum) {
    throw new WorkflowDraftPortError(
      `${label} must be non-empty trimmed text within ${maximum} characters`
    );
  }
  return value;
};

const parseResponse = (value: unknown): WorkflowDraftPortResult => {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    throw new WorkflowDraftPortError('Workflow draft response must be an object');
  }
  const record = value as Record<string, unknown>;
  if (
    Object.keys(record).length !== 1 ||
    !Object.prototype.hasOwnProperty.call(record, 'text') ||
    typeof record.text !== 'string' ||
    !record.text.trim() ||
    record.text.length > MAX_RESPONSE_UTF8_BYTES ||
    new TextEncoder().encode(record.text).byteLength > MAX_RESPONSE_UTF8_BYTES
  ) {
    throw new WorkflowDraftPortError(
      'Workflow draft response must contain only bounded non-empty text'
    );
  }
  return { text: record.text };
};

/**
 * One tool-less, non-persistent Chat request through NomiFun's authenticated
 * Creative Studio backend. The backend owns exact provider/model resolution.
 */
export const createWorkflowDraftPort = (
  request: WorkflowDraftHttpRequest = defaultRequest
): WorkflowDraftPort => ({
  async draft(input) {
    if (!CANONICAL_UUID_V7.test(input.providerId)) {
      throw new WorkflowDraftPortError('providerId must be a canonical UUIDv7');
    }
    const prompt = canonicalText(input.prompt, 'prompt', MAX_PROMPT_CHARACTERS);
    const model = canonicalText(input.model, 'model', MAX_MODEL_CHARACTERS);
    return parseResponse(
      await request('POST', CREATIVE_STUDIO_WORKFLOW_DRAFTS_ENDPOINT, {
        prompt,
        model: {
          providerId: input.providerId,
          model,
        },
      })
    );
  },
});

export const workflowDraftPort = createWorkflowDraftPort();
