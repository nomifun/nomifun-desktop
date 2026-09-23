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
