/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ToolReceiptDetailRow } from './components/toolGroupSummaryModel';

export const isFileReceiptRow = (row: ToolReceiptDetailRow): boolean =>
  (row.action === 'read_files' || row.action === 'edit_files') && Boolean(row.target);

export const shouldShowFileListDetail = (rows: ToolReceiptDetailRow[]): boolean =>
  rows.filter(isFileReceiptRow).length > 1;

export const shouldShowToolRowDetail = (
  row: ToolReceiptDetailRow,
  _options: { fileRowCount?: number } = {}
): boolean => {
  if (row.attempts?.length) return true;
  if (row.action === 'run_commands') return true;

  if (isFileReceiptRow(row)) {
    // Even one successful file receipt stays compact by default, but can be
    // expanded to reveal its exact path (and diagnostics for failures).
    return true;
  }

  return Boolean(row.input || row.output || row.truncated);
};

/**
 * Private thinking snapshots and routine model lifecycle updates do not
 * describe completed work. Keep at most the current live activity at the tail
 * of the journal; all public narration and tool receipts retain their order.
 */
export const selectJournalProcessItems = <T>(
  items: T[],
  options: {
    running: boolean;
    isPrivateActivity: (item: T) => boolean;
    isRunning: (item: T) => boolean;
    textOf?: (item: T) => string | undefined;
    finalText?: string;
  }
): T[] =>
  items.filter((item, index) => {
    const text = options.textOf?.(item);
    if (!options.running && text !== undefined && isProcessTextEchoOfFinal(text, options.finalText)) return false;
    return !options.isPrivateActivity(item) ||
      (options.running && index === items.length - 1 && options.isRunning(item));
  });

/**
 * Some model adapters emit an XML-shaped tool invocation as ordinary text
 * after a rejected tool call. It is neither public progress nor a successful
 * file result. Keep surrounding narration, but never expand the arguments or
 * embedded file content into the transcript. A fenced example stays visible.
 */
export const projectAssistantText = (content: string): { text: string; hasToolPayload: boolean } => {
  if (!/<tool_call>/i.test(content)) return { text: content.trim(), hasToolPayload: false };
  const lines = content.split(/\r?\n/);
  const visible: string[] = [];
  let fence: { marker: string; length: number } | undefined;
  let inToolPayload = false;
  let removed = false;

  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index];
    if (inToolPayload) {
      const closingTag = line.match(/<\/tool_call>/i);
      if (closingTag) {
        inToolPayload = false;
        const after = line.slice((closingTag.index ?? 0) + closingTag[0].length).trimStart();
        if (after) visible.push(after);
      }
      continue;
    }

    const fenceMatch = line.match(/^ {0,3}(`{3,}|~{3,})/);
    if (fenceMatch) {
      const marker = fenceMatch[1][0];
      if (!fence) fence = { marker, length: fenceMatch[1].length };
      else if (marker === fence.marker && fenceMatch[1].length >= fence.length) fence = undefined;
    }

    const openingTag = !fence ? line.match(/<tool_call>/i) : null;
    if (openingTag) {
      const before = line.slice(0, openingTag.index).trimEnd();
      const after = line.slice((openingTag.index ?? 0) + openingTag[0].length).trim();
      const nextNonEmpty = after || lines.slice(index + 1, index + 5).find((candidate) => candidate.trim());
      if (nextNonEmpty && /^<function=[^>\n]+>/i.test(nextNonEmpty.trim())) {
        if (before) visible.push(before);
        if (visible.length && visible.at(-1)?.trim()) visible.push('');
        inToolPayload = true;
        removed = true;
        continue;
      }
    }
    visible.push(line);
  }

  const result = visible.join('\n');
  return { text: (removed ? result.replace(/\n{3,}/g, '\n\n') : result).trim(), hasToolPayload: removed };
};

export const stripInternalToolCallPayload = (content: string): string => projectAssistantText(content).text;

/** One completed turn should not show its final answer twice. */
export const isProcessTextEchoOfFinal = (processText: string, finalText?: string): boolean => {
  if (!finalText) return false;
  const process = projectAssistantText(processText);
  const final = projectAssistantText(finalText);
  if (process.hasToolPayload || final.hasToolPayload) return false;
  const normalize = (value: string) => value.replace(/\s+/g, ' ').trim();
  return Boolean(process.text) && normalize(process.text) === normalize(final.text);
};

/** A model can repeat the same progress sentence after each tool boundary. */
export const deduplicateProcessText = <T>(
  items: T[],
  textOf: (item: T) => string | undefined
): T[] => {
  const seen = new Set<string>();
  let removed = false;
  const visible = items.filter((item) => {
    const text = textOf(item)?.replace(/\s+/g, ' ').trim();
    if (!text) return true;
    if (seen.has(text)) {
      removed = true;
      return false;
    }
    seen.add(text);
    return true;
  });
  return removed ? visible : items;
};

/**
 * Intermediate assistant prose and private-reasoning snapshots may arrive once
 * per model step. The transcript needs at most the latest public status for
 * each narration kind; tool/file receipts remain untouched and authoritative.
 */
export const collapseProcessNarration = <T>(
  items: T[],
  kindOf: (item: T) => 'text' | 'thinking' | undefined
): T[] => {
  const lastIndex = new Map<'text' | 'thinking', number>();
  items.forEach((item, index) => {
    const kind = kindOf(item);
    if (kind) lastIndex.set(kind, index);
  });
  if (lastIndex.size === 0) return items;
  const visible = items.filter((item, index) => {
    const kind = kindOf(item);
    return !kind || lastIndex.get(kind) === index;
  });
  return visible.length === items.length ? items : visible;
};
