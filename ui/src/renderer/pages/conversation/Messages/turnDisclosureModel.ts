/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

export type TurnDisclosureRole = 'user' | 'assistant' | 'process' | 'other';
export type TurnDisclosureProcessState = 'completed' | 'running' | 'waiting' | 'failed' | 'canceled';

export interface TurnDisclosureInputItem {
  id: string;
  turnId?: string;
  role: TurnDisclosureRole;
  createdAt: number;
  processState?: TurnDisclosureProcessState;
  running?: boolean;
  sourceMessageIds?: string[];
}

export type TurnDisclosureOutputItem =
  | { type: 'item'; id: string }
  | { type: 'process_receipt'; id: string; itemId: string }
  | {
      type: 'turn_disclosure';
      id: string;
      turnId: string;
      processItemIds: string[];
      sourceMessageIds: string[];
      startAt: number;
      endAt: number;
      state: TurnDisclosureProcessState;
      running: boolean;
      defaultCollapsed: boolean;
    };

const unique = (values: string[]): string[] => Array.from(new Set(values.filter(Boolean)));

const getProcessState = (entry: TurnDisclosureInputItem): TurnDisclosureProcessState => {
  if (entry.processState) return entry.processState;
  if (entry.running) return 'running';
  return 'completed';
};

const resolveDisclosureState = (processItems: TurnDisclosureInputItem[]): TurnDisclosureProcessState => {
  const states = processItems.map(getProcessState);
  if (states.includes('waiting')) return 'waiting';
  if (states.includes('running')) return 'running';
  if (states.includes('failed')) return 'failed';
  if (states.includes('canceled')) return 'canceled';
  return 'completed';
};

function buildSegmentOutput(segment: TurnDisclosureInputItem[]): TurnDisclosureOutputItem[] {
  const turnId = segment[0]?.turnId;
  if (!turnId) return segment.map((entry) => ({ type: 'item', id: entry.id }));

  const finalAssistantIndex = segment.findLastIndex((entry) => entry.role === 'assistant');

  const processItems = segment.filter((entry, index) => {
    if (entry.role === 'user' || entry.role === 'other') return false;
    return index !== finalAssistantIndex;
  });

  if (!processItems.length) {
    return segment.map((entry) => ({ type: 'item', id: entry.id }));
  }

  const state = resolveDisclosureState(processItems);
  if (state === 'running' || state === 'waiting') {
    return segment.map((entry) => {
      if (entry.role === 'process') {
        return { type: 'process_receipt', id: `receipt-${entry.id}`, itemId: entry.id };
      }
      return { type: 'item', id: entry.id };
    });
  }

  const finalOrProcessItems =
    finalAssistantIndex === -1 ? processItems : [...processItems, segment[finalAssistantIndex]].filter(Boolean);
  const disclosure: TurnDisclosureOutputItem = {
    type: 'turn_disclosure',
    id: `turn-disclosure-${turnId}`,
    turnId,
    processItemIds: processItems.map((entry) => entry.id),
    sourceMessageIds: unique(processItems.flatMap((entry) => entry.sourceMessageIds ?? [entry.id])),
    startAt: Math.min(...processItems.map((entry) => entry.createdAt)),
    endAt: Math.max(...finalOrProcessItems.map((entry) => entry.createdAt)),
    state,
    running: false,
    defaultCollapsed: true,
  };

  const output: TurnDisclosureOutputItem[] = [];
  let insertedDisclosure = false;

  segment.forEach((entry, index) => {
    if (entry.role !== 'user' && entry.role !== 'other' && index !== finalAssistantIndex) {
      return;
    }

    if (index === finalAssistantIndex && !insertedDisclosure) {
      output.push(disclosure);
      insertedDisclosure = true;
    }

    output.push({ type: 'item', id: entry.id });
  });

  if (!insertedDisclosure) {
    output.push(disclosure);
  }

  return output;
}

export function buildTurnDisclosureItems(items: TurnDisclosureInputItem[]): TurnDisclosureOutputItem[] {
  const output: TurnDisclosureOutputItem[] = [];
  let segment: TurnDisclosureInputItem[] = [];

  const flush = () => {
    if (!segment.length) return;
    output.push(...buildSegmentOutput(segment));
    segment = [];
  };

  for (const item of items) {
    if (!item.turnId) {
      flush();
      output.push({ type: 'item', id: item.id });
      continue;
    }

    const currentTurnId = segment[0]?.turnId;
    if (currentTurnId && currentTurnId !== item.turnId) {
      flush();
    }

    segment.push(item);
  }

  flush();
  return output;
}
