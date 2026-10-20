/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  type NormalizedToolCall,
  type NormalizedToolNotExecutedReason,
} from '@/common/chat/normalizeToolCall';
import type { TurnDisclosureProcessState } from '../turnDisclosureModel';
import { formatToolPresentationLabel, resolveToolPresentation } from '@/common/chat/toolPresentation';

export type ToolReceiptAction =
  | 'read_files'
  | 'edit_files'
  | 'run_commands'
  | 'search_code'
  | 'discover_tools'
  | 'generic';

export type ToolReceiptIcon = 'tool' | 'file' | 'edit';

export interface ToolReceiptRow {
  key: string;
  action: ToolReceiptAction;
  state: TurnDisclosureProcessState;
  title: string;
  target?: string;
  error?: string;
  skipped?: boolean;
  notExecutedReason?: NormalizedToolNotExecutedReason;
  commandExitCode?: number;
  commandNotStarted?: boolean;
  commandTimedOut?: boolean;
}

export const countBoundedSearchResults = (tools: NormalizedToolCall[]): number =>
  tools.filter((tool) => tool.boundedResult === 'search_context_withheld').length;

const toolReceiptIconByAction: Record<ToolReceiptAction, ToolReceiptIcon> = {
  read_files: 'file',
  edit_files: 'edit',
  run_commands: 'tool',
  search_code: 'file',
  discover_tools: 'tool',
  generic: 'tool',
};

const compactToolText = (value?: string): string => value?.replace(/\s+/g, ' ').trim() ?? '';

/** Display categories come from the shared admitted catalogue, never arbitrary aliases. */
const classifyToolForReceipt = (tool: NormalizedToolCall): ToolReceiptAction => {
  const action = resolveToolPresentation(tool).receiptAction;
  return action && Object.hasOwn(toolReceiptIconByAction, action) ? action as ToolReceiptAction : 'generic';
};

const getFileTarget = (tool: NormalizedToolCall): string | undefined => {
  const description = compactToolText(tool.description);
  return description && description !== tool.name ? description : undefined;
};

const getToolReceiptDetailTarget = (tool: NormalizedToolCall, action: ToolReceiptAction, language: string): string | undefined => {
  const presentation = resolveToolPresentation(tool, language);
  if (action === 'generic' || action === 'discover_tools') return formatToolPresentationLabel(presentation);
  if (action === 'read_files' || action === 'edit_files') return getFileTarget(tool);
  return compactToolText(tool.description) || undefined;
};

const getToolProcessState = (tool: NormalizedToolCall): TurnDisclosureProcessState => {
  if (tool.status === 'running' || tool.status === 'pending') return 'running';
  // Receipt state controls color and turn health. The dedicated copy carries
  // the "not executed" meaning without presenting a local preflight rejection
  // as a user-visible task failure/cancellation.
  if (tool.notExecutedReason) return 'completed';
  if (tool.nonFatalFailure) return 'completed';
  if (tool.status === 'error') return 'failed';
  if (tool.status === 'canceled') return 'canceled';
  return 'completed';
};

/** Icons and row summaries use the same projection, without a second grouping ledger. */
export const getToolReceiptIcon = (rows: ToolReceiptRow[]): ToolReceiptIcon | undefined => {
  const focused = rows.findLast(row => row.state === 'running')
    ?? rows.findLast(row => row.state === 'failed' || row.state === 'canceled') ?? rows.at(-1);
  return focused ? toolReceiptIconByAction[focused.action] : undefined;
};

export const buildToolReceiptRows = (tools: NormalizedToolCall[], language = 'en-US'): ToolReceiptRow[] =>
  tools.map((tool) => {
    const action = classifyToolForReceipt(tool);
    const presentation = resolveToolPresentation(tool, language);
    const title = presentation.title;
    const target = getToolReceiptDetailTarget(tool, action, language);
    return {
      key: tool.key,
      action,
      state: getToolProcessState(tool),
      title,
      ...(target ? { target } : {}),
      ...(tool.error ? { error: tool.error } : {}),
      ...(tool.skipped ? { skipped: true } : {}),
      ...(tool.notExecutedReason ? { notExecutedReason: tool.notExecutedReason } : {}),
      ...(tool.commandExitCode !== undefined ? { commandExitCode: tool.commandExitCode } : {}),
      ...(tool.commandNotStarted ? { commandNotStarted: true } : {}),
      ...(tool.commandTimedOut ? { commandTimedOut: true } : {}),

    };
  });
