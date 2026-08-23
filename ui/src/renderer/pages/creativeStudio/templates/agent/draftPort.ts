/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { httpRequest } from '@/common/adapter/httpBridge';
import { CANONICAL_UUID_V7, type ProviderId } from '@/common/types/ids';

import { MAX_CREATIVE_TEMPLATE_DRAFT_JSON_BYTES } from './artifacts';

export const CREATIVE_STUDIO_TEMPLATE_DRAFTS_ENDPOINT =
  '/api/creative-studio/template-drafts';

const MAX_PROMPT_CHARACTERS = 20_000;
const MAX_MODEL_CHARACTERS = 512;
const MAX_RESPONSE_UTF8_BYTES = MAX_CREATIVE_TEMPLATE_DRAFT_JSON_BYTES + 12;

export interface TemplateDraftPortInput {
  providerId: ProviderId;
  model: string;
  prompt: string;
}

export interface TemplateDraftPortResult {
  text: string;
}

export interface TemplateDraftPort {
  draft(input: TemplateDraftPortInput): Promise<TemplateDraftPortResult>;
}

export type TemplateDraftHttpRequest = (
  method: string,
  path: string,
  body: unknown
) => Promise<unknown>;

export class TemplateDraftPortError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'TemplateDraftPortError';
  }
}

const defaultRequest: TemplateDraftHttpRequest = (method, path, body) =>
  httpRequest<unknown>(method, path, body, { timeoutMs: 125_000 });

const canonicalText = (
  value: string,
  label: string,
  maximum: number
): string => {
  if (!value || value !== value.trim() || value.length > maximum) {
    throw new TemplateDraftPortError(
      `${label} must be non-empty trimmed text within ${maximum} characters`
    );
  }
  return value;
};

const parseResponse = (value: unknown): TemplateDraftPortResult => {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    throw new TemplateDraftPortError('Template draft response must be an object');
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
    throw new TemplateDraftPortError(
      'Template draft response must contain only bounded non-empty text'
    );
  }
  return { text: record.text };
};

/**
 * One tool-less, non-persistent Chat request through NomiFun's authenticated
 * Creative Studio backend. The backend owns exact provider/model resolution.
 */
export const createTemplateDraftPort = (
  request: TemplateDraftHttpRequest = defaultRequest
): TemplateDraftPort => ({
  async draft(input) {
    if (!CANONICAL_UUID_V7.test(input.providerId)) {
      throw new TemplateDraftPortError('providerId must be a canonical UUIDv7');
    }
    const prompt = canonicalText(input.prompt, 'prompt', MAX_PROMPT_CHARACTERS);
    const model = canonicalText(input.model, 'model', MAX_MODEL_CHARACTERS);
    return parseResponse(
      await request('POST', CREATIVE_STUDIO_TEMPLATE_DRAFTS_ENDPOINT, {
        prompt,
        model: {
          providerId: input.providerId,
          model,
        },
      })
    );
  },
});

export const templateDraftPort = createTemplateDraftPort();
