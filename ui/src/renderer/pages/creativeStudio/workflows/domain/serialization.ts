/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  WorkflowExportResult,
  WorkflowParseResult,
  WorkflowWorkspaceDocumentV1,
} from './types';
import { WORKFLOW_LIMITS, validateWorkflowWorkspaceDocument } from './validation';

export function parseWorkflowWorkspaceV1(json: string): WorkflowParseResult {
  if (typeof json !== 'string' || new TextEncoder().encode(json).byteLength > WORKFLOW_LIMITS.jsonBytes) {
    return {
      ok: false,
      error: { code: 'limit-exceeded', path: '$', message: 'workflow JSON exceeds the v1 limit' },
    };
  }
  let value: unknown;
  try {
    value = JSON.parse(json) as unknown;
  } catch {
    return {
      ok: false,
      error: { code: 'invalid-json', path: '$', message: 'workflow document is not valid JSON' },
    };
  }
  const validation = validateWorkflowWorkspaceDocument(value);
  return validation.ok
    ? { ok: true, document: value as WorkflowWorkspaceDocumentV1 }
    : validation;
}

export function exportWorkflowWorkspaceV1(
  document: WorkflowWorkspaceDocumentV1
): WorkflowExportResult {
  const validation = validateWorkflowWorkspaceDocument(document);
  if (!validation.ok) return validation;
  const json = JSON.stringify(document);
  if (new TextEncoder().encode(json).byteLength > WORKFLOW_LIMITS.jsonBytes) {
    return {
      ok: false,
      error: { code: 'limit-exceeded', path: '$', message: 'workflow JSON exceeds the v1 limit' },
    };
  }
  return { ok: true, json, document };
}
