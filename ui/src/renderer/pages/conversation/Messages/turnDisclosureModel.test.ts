/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { buildTurnDisclosureItems, type TurnDisclosureInputItem } from './turnDisclosureModel';

const item = (
  id: string,
  role: TurnDisclosureInputItem['role'],
  options: Partial<TurnDisclosureInputItem> = {}
): TurnDisclosureInputItem => ({
  id,
  turnId: 'turn-1',
  role,
  createdAt: options.createdAt ?? 1000,
  sourceMessageIds: options.sourceMessageIds ?? [id],
  ...options,
});

describe('buildTurnDisclosureItems', () => {
  test('collapses completed intermediate steps into a disclosure before the final answer', () => {
    const result = buildTurnDisclosureItems([
      item('user', 'user', { createdAt: 1000 }),
      item('thinking', 'process', { createdAt: 2000 }),
      item('tool', 'process', { createdAt: 3000 }),
      item('final', 'assistant', { createdAt: 5000 }),
    ]);

    expect(result.map((entry) => entry.type === 'item' ? entry.id : entry.id)).toEqual([
      'user',
      'turn-disclosure-turn-1',
      'final',
    ]);

    const disclosure = result[1];
    expect(disclosure.type).toBe('turn_disclosure');
    if (disclosure.type !== 'turn_disclosure') return;
    expect(disclosure.defaultCollapsed).toBe(true);
    expect(disclosure.state).toBe('completed');
    expect(disclosure.processItemIds).toEqual(['thinking', 'tool']);
    expect(disclosure.startAt).toBe(2000);
    expect(disclosure.endAt).toBe(5000);
    expect(disclosure.sourceMessageIds).toEqual(['thinking', 'tool']);
  });

  test('keeps the final assistant answer outside the disclosure when earlier assistant text was intermediate', () => {
    const result = buildTurnDisclosureItems([
      item('user', 'user', { createdAt: 1000 }),
      item('analysis-note', 'assistant', { createdAt: 1500 }),
      item('tool', 'process', { createdAt: 2000 }),
      item('summary', 'assistant', { createdAt: 4000 }),
    ]);

    const disclosure = result.find((entry) => entry.type === 'turn_disclosure');
    expect(disclosure?.processItemIds).toEqual(['analysis-note', 'tool']);
    expect(result.map((entry) => entry.type === 'item' ? entry.id : entry.id)).toEqual([
      'user',
      'turn-disclosure-turn-1',
      'summary',
    ]);
  });

  test('renders unfinished running process steps as inline receipts before the final answer exists', () => {
    const result = buildTurnDisclosureItems([
      item('user', 'user', { createdAt: 1000 }),
      item('thinking', 'process', { createdAt: 2000, processState: 'running' }),
      item('tool', 'process', { createdAt: 3000 }),
    ]);

    expect(result).toEqual([
      { type: 'item', id: 'user' },
      { type: 'process_receipt', id: 'receipt-thinking', itemId: 'thinking' },
      { type: 'process_receipt', id: 'receipt-tool', itemId: 'tool' },
    ]);
  });

  test('keeps running assistant text visible and renders process steps as receipts', () => {
    const result = buildTurnDisclosureItems([
      item('user', 'user', { createdAt: 1000 }),
      item('progress-note', 'assistant', { createdAt: 1500 }),
      item('thinking', 'process', { createdAt: 2000, processState: 'running' }),
      item('partial-answer', 'assistant', { createdAt: 3000 }),
    ]);

    expect(result).toEqual([
      { type: 'item', id: 'user' },
      { type: 'item', id: 'progress-note' },
      { type: 'process_receipt', id: 'receipt-thinking', itemId: 'thinking' },
      { type: 'item', id: 'partial-answer' },
    ]);
  });

  test('keeps waiting confirmation steps visible as inline receipts', () => {
    const result = buildTurnDisclosureItems([
      item('user', 'user', { createdAt: 1000 }),
      item('permission', 'process', { createdAt: 2000, processState: 'waiting' }),
      item('partial-answer', 'assistant', { createdAt: 3000 }),
    ]);

    expect(result).toEqual([
      { type: 'item', id: 'user' },
      { type: 'process_receipt', id: 'receipt-permission', itemId: 'permission' },
      { type: 'item', id: 'partial-answer' },
    ]);
  });

  test('surfaces failed process state on a completed disclosure', () => {
    const result = buildTurnDisclosureItems([
      item('user', 'user', { createdAt: 1000 }),
      item('tool', 'process', { createdAt: 2000, processState: 'failed' }),
      item('final', 'assistant', { createdAt: 3000 }),
    ]);

    const disclosure = result[1];
    expect(disclosure.type).toBe('turn_disclosure');
    if (disclosure.type !== 'turn_disclosure') return;
    expect(disclosure.defaultCollapsed).toBe(true);
    expect(disclosure.state).toBe('failed');
  });

  test('collapses completed process-only turns when no final assistant answer exists', () => {
    const result = buildTurnDisclosureItems([
      item('user', 'user', { createdAt: 1000 }),
      item('tool', 'process', { createdAt: 2000, processState: 'completed' }),
    ]);

    expect(result.map((entry) => (entry.type === 'item' ? entry.id : entry.id))).toEqual([
      'user',
      'turn-disclosure-turn-1',
    ]);

    const disclosure = result[1];
    expect(disclosure.type).toBe('turn_disclosure');
    if (disclosure.type !== 'turn_disclosure') return;
    expect(disclosure.defaultCollapsed).toBe(true);
    expect(disclosure.state).toBe('completed');
    expect(disclosure.processItemIds).toEqual(['tool']);
  });

  test('keeps completed disclosures scoped to their own turn id', () => {
    const result = buildTurnDisclosureItems([
      item('user-1', 'user', { turnId: 'turn-1', createdAt: 1000 }),
      item('tool-1', 'process', { turnId: 'turn-1', createdAt: 2000 }),
      item('final-1', 'assistant', { turnId: 'turn-1', createdAt: 3000 }),
      item('user-2', 'user', { turnId: 'turn-2', createdAt: 4000 }),
      item('tool-2', 'process', { turnId: 'turn-2', createdAt: 5000 }),
      item('final-2', 'assistant', { turnId: 'turn-2', createdAt: 6000 }),
    ]);

    expect(result.map((entry) => (entry.type === 'item' ? entry.id : entry.id))).toEqual([
      'user-1',
      'turn-disclosure-turn-1',
      'final-1',
      'user-2',
      'turn-disclosure-turn-2',
      'final-2',
    ]);
    expect(result.filter((entry) => entry.type === 'turn_disclosure')).toHaveLength(2);
  });
});
