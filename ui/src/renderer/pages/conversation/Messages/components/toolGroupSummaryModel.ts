/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  formatToolDisplayName,
  type NormalizedToolCall,
  type NormalizedToolNotExecutedReason,
} from '@/common/chat/normalizeToolCall';
import type { TurnDisclosureProcessState } from '../turnDisclosureModel';
import { mergeProcessStates } from '../turnProcessState';
import { formatToolPresentationLabel, resolveToolPresentation } from '@/common/chat/toolPresentation';

export type ToolReceiptAction =
  | 'read_files'
  | 'edit_files'
  | 'run_commands'
  | 'search_code'
  | 'list_files'
  | 'discover_tools'
  | 'generic';

export type ToolReceiptIcon = 'tool' | 'file' | 'edit';

export interface ToolReceiptSummaryPart {
  action: ToolReceiptAction;
  /** Number of calls, never the number of matched or loaded schemas. */
  count: number;
  state: TurnDisclosureProcessState;
  target?: string;
  skipped?: boolean;
  notExecutedReason?: NormalizedToolNotExecutedReason;
  commandExitCode?: number;
  commandNotStarted?: boolean;
  commandTimedOut?: boolean;
}

export interface ToolReceiptDetailRow {
  key: string;
  action: ToolReceiptAction;
  state: TurnDisclosureProcessState;
  title: string;
  /** Preserve the existing named-action label without exposing routing details. */
  showActionTitle?: boolean;
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
  list_files: 'file',
  discover_tools: 'tool',
  generic: 'tool',
};

const compactToolText = (value?: unknown): string => {
  if (value == null) return '';
  const text =
    typeof value === 'string'
      ? value
      : (() => {
          try {
            return JSON.stringify(value, null, 2);
          } catch {
            return String(value);
          }
        })();
  return text.replace(/\s+/g, ' ').trim();
};

const formatToolTarget = (tool: NormalizedToolCall, language = 'en-US'): string => {
  if (classifyToolForReceipt(tool) === 'run_commands') return getCommandTarget(tool);
  return formatToolPresentationLabel(resolveToolPresentation(tool, language));
};

const getCommandTarget = (tool: NormalizedToolCall): string =>
  compactToolText(tool.description) || compactToolText(formatToolDisplayName(tool.name)) || tool.key;

const getFileTarget = (tool: NormalizedToolCall): string | undefined => {
  const description = compactToolText(tool.description);
  const name = compactToolText(formatToolDisplayName(tool.name));
  return description && description !== name && description !== tool.name ? description : undefined;
};

const normalizeToolSearchText = (value: string): string => value.replace(/[_-]+/g, ' ').toLowerCase();

// Receipt categories describe concrete UI affordances, so classify only tool
// names whose semantics we actually know. Keyword matching arbitrary MCP names
// (for example `nomi_knowledge_update_base`) incorrectly presented domain
// operations as file edits.
const commandToolNames = new Set(['bash', 'shell', 'exec', 'exec command', 'execute command', 'run command', 'terminal']);
const codeSearchToolNames = new Set(['grep', 'rg', 'search', 'find']);
const fileListToolNames = new Set(['glob', 'list', 'ls', 'directory', 'dir']);
const fileEditToolNames = new Set(['write', 'edit', 'patch', 'apply patch', 'multi edit', 'replace']);
const fileReadToolNames = new Set(['read', 'open', 'view', 'cat']);

const commandActionTokens = new Set(['exec', 'execute', 'run']);
const commandSuffixes = new Set(['command', 'commands']);
const codeSearchActionTokens = new Set(['grep', 'rg', 'search', 'find']);
const codeSearchSuffixes = new Set(['code', 'file', 'files']);
const fileListActionTokens = new Set(['glob', 'list', 'ls']);
const fileListSuffixes = new Set(['file', 'files', 'directory', 'directories', 'entry', 'entries']);
const fileEditActionTokens = new Set(['write', 'edit', 'patch', 'replace', 'apply', 'multi']);
const fileEditSuffixes = new Set(['file', 'files', 'patch', 'edit']);
const fileReadActionTokens = new Set(['read', 'open', 'view', 'cat']);
const fileReadSuffixes = new Set(['file', 'files']);

interface ToolActionName {
  name: string;
  isMcp: boolean;
}

const getNamespacedToolActionName = (value?: unknown): ToolActionName => {
  const fullName = compactToolText(value);
  const segments = fullName.split('__');
  const isMcp = fullName.startsWith('mcp__');
  // Stable MCP provider names end in a 16-character base32 origin hash. It is
  // routing metadata, not the tool action; remove only that exact canonical
  // suffix so legacy/non-MCP names cannot be reinterpreted by accident.
  if (isMcp && segments.length >= 4 && /^[a-z2-7]{16}$/.test(segments.at(-1) ?? '')) {
    segments.pop();
  }
  const leafName = segments.at(-1) ?? fullName;
  return { name: normalizeToolSearchText(leafName).trim(), isMcp };
};

const matchesAnchoredToolAction = (
  name: string,
  exactNames: Set<string>,
  actionTokens: Set<string>,
  suffixes: Set<string>,
  allowExactName: boolean
): boolean => {
  if (allowExactName && exactNames.has(name)) return true;
  const [action, ...suffixParts] = name.split(' ').filter(Boolean);
  return actionTokens.has(action) && suffixParts.length > 0 && suffixes.has(suffixParts.join(' '));
};

const classifyToolForReceipt = (tool: NormalizedToolCall): ToolReceiptAction => {
  const { name, isMcp } = getNamespacedToolActionName(tool.name);

  const explicitAction = resolveToolPresentation(tool).receiptAction;
  if (explicitAction) return explicitAction as ToolReceiptAction;
  // A server-local MCP name is descriptive metadata, not a trusted semantic
  // kind. Preserve strong compound actions such as read_file/exec_command, but
  // do not turn ambiguous one-word names such as search/read/run/list into
  // code, file, or command receipts. Native built-ins keep their exact-name UI.
  const allowExactName = !isMcp;
  if (
    matchesAnchoredToolAction(
      name,
      commandToolNames,
      commandActionTokens,
      commandSuffixes,
      allowExactName
    )
  ) {
    return 'run_commands';
  }
  if (
    matchesAnchoredToolAction(
      name,
      codeSearchToolNames,
      codeSearchActionTokens,
      codeSearchSuffixes,
      allowExactName
    )
  ) {
    return 'search_code';
  }
  if (
    matchesAnchoredToolAction(
      name,
      fileListToolNames,
      fileListActionTokens,
      fileListSuffixes,
      allowExactName
    )
  ) {
    return 'list_files';
  }
  if (
    matchesAnchoredToolAction(
      name,
      fileEditToolNames,
      fileEditActionTokens,
      fileEditSuffixes,
      allowExactName
    )
  ) {
    return 'edit_files';
  }
  if (
    matchesAnchoredToolAction(
      name,
      fileReadToolNames,
      fileReadActionTokens,
      fileReadSuffixes,
      allowExactName
    )
  ) {
    return 'read_files';
  }
  return 'generic';
};

const getToolReceiptTarget = (tool: NormalizedToolCall, action: ToolReceiptAction, language: string): string | undefined => {
  if (action === 'run_commands') {
    return getCommandTarget(tool);
  }
  if (action === 'read_files' || action === 'edit_files') {
    return getFileTarget(tool);
  }
  if (action === 'discover_tools') return formatToolTarget(tool, language);
  if (action !== 'generic') return undefined;
  return formatToolTarget(tool, language);
};

const getToolReceiptDetailTarget = (tool: NormalizedToolCall, action: ToolReceiptAction, language: string): string | undefined => {
  const description = compactToolText(tool.description);
  const rawName = compactToolText(tool.name);
  const name = compactToolText(formatToolDisplayName(rawName));

  if (action === 'generic' || action === 'discover_tools') return formatToolTarget(tool, language);
  if (action === 'read_files' || action === 'edit_files') return getFileTarget(tool);
  if (description && description !== name && description !== rawName) return description;
  if (action === 'run_commands') return getCommandTarget(tool);
  return undefined;
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

export const buildToolReceiptSummaryParts = (
  tools: NormalizedToolCall[],
  _state: TurnDisclosureProcessState,
  language = 'en-US'
): ToolReceiptSummaryPart[] => {
  const grouped = new Map<
    string,
    {
      action: ToolReceiptAction;
      count: number;
      skippedCount: number;
      targets: string[];
      states: TurnDisclosureProcessState[];
      notExecutedReason?: NormalizedToolNotExecutedReason;
      commandExitCode?: number;
      commandNotStarted?: boolean;
      commandTimedOut?: boolean;
    }
  >();

  tools.forEach((tool) => {
    const action = classifyToolForReceipt(tool);
    const target = getToolReceiptTarget(tool, action, language);
    // A pre-dispatch rejection is a different receipt outcome from a tool that
    // actually ran. Keep them separate even when their semantic action matches.
    const groupKey = `${action}:${tool.notExecutedReason ?? 'executed'}:${tool.commandExitCode ?? 'unknown'}:${tool.commandNotStarted === true}:${tool.commandTimedOut === true}`;
    const current = grouped.get(groupKey) ?? {
      action,
      count: 0,
      skippedCount: 0,
      targets: [],
      states: [],
      ...(tool.notExecutedReason ? { notExecutedReason: tool.notExecutedReason } : {}),
      ...(tool.commandExitCode !== undefined ? { commandExitCode: tool.commandExitCode } : {}),
      ...(tool.commandNotStarted ? { commandNotStarted: true } : {}),
      ...(tool.commandTimedOut ? { commandTimedOut: true } : {}),
    };
    current.count += 1;
    if (tool.skipped) current.skippedCount += 1;
    current.states.push(getToolProcessState(tool));
    if (target) current.targets.push(target);
    grouped.set(groupKey, current);
  });

  return Array.from(grouped.values()).map((value) => ({
    action: value.action,
    count: value.count,
    state: mergeProcessStates(value.states),
    ...(value.targets.length ? { target: Array.from(new Set(value.targets)).join(', ') } : {}),
    ...(value.skippedCount === value.count ? { skipped: true } : {}),
    ...(value.notExecutedReason ? { notExecutedReason: value.notExecutedReason } : {}),
    ...(value.commandExitCode !== undefined ? { commandExitCode: value.commandExitCode } : {}),
    ...(value.commandNotStarted ? { commandNotStarted: true } : {}),
    ...(value.commandTimedOut ? { commandTimedOut: true } : {}),
  }));
};

export const getToolReceiptIconFromSummaryParts = (parts: ToolReceiptSummaryPart[]): ToolReceiptIcon | undefined => {
  const focusedPart =
    parts.findLast((part) => part.state === 'running') ??
    parts.findLast((part) => part.state === 'failed' || part.state === 'canceled') ??
    parts.at(-1);
  return focusedPart ? toolReceiptIconByAction[focusedPart.action] : undefined;
};

export const buildToolReceiptDetailRows = (tools: NormalizedToolCall[], language = 'en-US'): ToolReceiptDetailRow[] =>
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
      ...(action !== 'generic' && (presentation.title !== tool.name || presentation.source || tool.capabilityId || tool.actionId)
        ? { showActionTitle: true } : {}),
      ...(target ? { target } : {}),
      ...(tool.error ? { error: tool.error } : {}),
      ...(tool.skipped ? { skipped: true } : {}),
      ...(tool.notExecutedReason ? { notExecutedReason: tool.notExecutedReason } : {}),
      ...(tool.commandExitCode !== undefined ? { commandExitCode: tool.commandExitCode } : {}),
      ...(tool.commandNotStarted ? { commandNotStarted: true } : {}),
      ...(tool.commandTimedOut ? { commandTimedOut: true } : {}),

    };
  });
