/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  CreativeTemplateExportResult,
  CreativeTemplateParseResult,
  CreativeTemplateWorkspaceDocumentV1,
} from './types';
import { TEMPLATE_LIMITS, validateTemplateWorkspaceDocument } from './validation';

export function parseTemplateWorkspaceV1(json: string): CreativeTemplateParseResult {
  if (typeof json !== 'string' || new TextEncoder().encode(json).byteLength > TEMPLATE_LIMITS.jsonBytes) {
    return {
      ok: false,
      error: { code: 'limit-exceeded', path: '$', message: 'template JSON exceeds the v1 limit' },
    };
  }
  let value: unknown;
  try {
    value = JSON.parse(json) as unknown;
  } catch {
    return {
      ok: false,
      error: { code: 'invalid-json', path: '$', message: 'template document is not valid JSON' },
    };
  }
  const validation = validateTemplateWorkspaceDocument(value);
  return validation.ok
    ? { ok: true, document: value as CreativeTemplateWorkspaceDocumentV1 }
    : validation;
}

export function exportTemplateWorkspaceV1(
  document: CreativeTemplateWorkspaceDocumentV1
): CreativeTemplateExportResult {
  const validation = validateTemplateWorkspaceDocument(document);
  if (!validation.ok) return validation;
  const json = JSON.stringify(document);
  if (new TextEncoder().encode(json).byteLength > TEMPLATE_LIMITS.jsonBytes) {
    return {
      ok: false,
      error: { code: 'limit-exceeded', path: '$', message: 'template JSON exceeds the v1 limit' },
    };
  }
  return { ok: true, json, document };
}
