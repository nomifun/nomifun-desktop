/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const source = readFileSync(new URL('./MessageList.tsx', import.meta.url), 'utf8');

describe('MessageList turn completion disclosure structure', () => {
  test('routes message content through the turn disclosure model before rendering', () => {
    expect(source.includes('buildTurnDisclosureItems')).toBe(true);
    expect(source.includes("type: 'turn_process_disclosure'")).toBe(true);
    expect(source.includes('renderTurnDisclosure')).toBe(true);
    expect(source.includes('components/TurnProcessDisclosure')).toBe(true);
    expect(source.includes("type: 'process_receipt'")).toBe(true);
    expect(source.includes('renderProcessReceipt')).toBe(true);
    expect(source.includes('components/TurnProcessReceipt')).toBe(true);
    expect(source.includes('getProcessItemState')).toBe(true);
    expect(source.includes('highlighted={highlighted}')).toBe(true);
  });

  test('keeps the implementation scoped to the message content area', () => {
    expect(source.includes('PreviewPanel')).toBe(false);
    expect(source.includes('OrchestrationTopPanel')).toBe(false);
    expect(source.includes('ProjectedWorkerView')).toBe(false);
  });
});
