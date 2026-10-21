/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { MessageId } from '@/common/types/ids';
import type { AgentTurnOutcome } from '@/common/protocolBindings/AgentTurnOutcome';
import { mergeAgentTurnOutcome } from '@/common/chat/turnOutcome';

export type TurnDisclosureRole = 'user' | 'assistant' | 'process' | 'process_content' | 'metadata' | 'other';
export type TurnDisclosureProcessState = 'completed' | 'running' | 'failed' | 'canceled';

export interface TurnDisclosureInputItem {
  id: string;
  turnId?: MessageId;
  role: TurnDisclosureRole;
  /** Canonical input membership; steering never opens another Turn. */
  inputKind?: 'root' | 'steering';
  /** Canonical display boundary, independent of the wall clock. */
  sequence?: number;
  createdAt: number;
  /** Wall-clock content time; createdAt may instead be a stable history cursor. */
  displayAt?: number;
  processStartedAt?: number;
  processEndedAt?: number;
  processState?: TurnDisclosureProcessState;
  /** Canonical wall-clock interval supplied by the owning Turn projection. */
  turnStartedAt?: number;
  turnEndedAt?: number;
  /** Terminal assistant rows (notably errors) own the settled outcome. */
  terminal?: boolean;
  running?: boolean;
  sourceMessageIds?: MessageId[];
  publicText?: boolean;
  /** Host-derived public delivery receipt, never inferred from text order. */
  turnOutcome?: AgentTurnOutcome;
  userInputRequestId?: string;
  /** Explicit non-Runtime projection; absence of a canonical receipt is not this marker. */
  standalonePublic?: boolean;
}

export type TurnDisclosureOutputItem =
  | { type: 'item'; id: string }
  | { type: 'process_receipt'; id: string; itemId: string }
  | {
      type: 'turn_disclosure';
      id: string;
      turnId: MessageId;
      processItemIds: string[];
      sourceMessageIds: MessageId[];
      startAt: number;
      endAt: number;
      state: TurnDisclosureProcessState;
      processItemStates: Record<string, TurnDisclosureProcessState>;
      running: boolean;
      defaultCollapsed: boolean;
      earlierSegment?: boolean;
    };

export interface BuildTurnDisclosureOptions {
  /** Exact source-bound read projections may outlive a paged-out tool message. */
  processRecordTurnIds?: ReadonlySet<MessageId>;
  tailClosed?: boolean;
  activeTurnId?: MessageId;
  /** Canonical runtime start when the current history window omits the Turn summary. */
  activeTurnStartedAt?: number;
  /**
   * Present when the user stopped the latest turn in this session. The tail
   * disclosure (once closed) renders as canceled with `endAt` pinned to the
   * stop moment, so the header reads "you stopped after {duration}" even when
   * the backend never emitted a Canceled tool status.
   */
  stopNotice?: { stoppedAt: number };
}

export interface AssignTurnIdOptions {
  activeTurnId?: MessageId;
  activeRequestMessageId?: MessageId;
}

const unique = <T extends string>(values: T[]): T[] => Array.from(new Set(values.filter(Boolean)));

// REST optional timestamps can be JSON null; only finite numbers mark a Turn
// boundary. A missing finish must never settle an otherwise running Turn.
const isTurnTimestamp = (value: unknown): value is number =>
  typeof value === 'number' && Number.isFinite(value);

/** Public result membership is owned by the canonical Turn receipt. Unscoped
 * local projections keep their public content; they do not elect a final row. */
export function selectTurnPublicResultIds(items: TurnDisclosureInputItem[]): ReadonlySet<string> {
  const outcomes = new Map<MessageId, AgentTurnOutcome>();
  for (const item of items) if (item.turnId && item.turnOutcome) {
    outcomes.set(item.turnId, mergeAgentTurnOutcome(outcomes.get(item.turnId), item.turnOutcome)!);
  }
  const visible = new Set<string>();
  for (const item of items) {
    if (item.role !== 'assistant') continue;
    const outcome = item.turnId ? outcomes.get(item.turnId) : undefined;
    if (!outcome) {
      if (item.standalonePublic || item.terminal || item.userInputRequestId) visible.add(item.id);
      continue;
    }
    switch (outcome.kind) {
      case 'silent':
        // A deliberate no-reply terminal still retains its process disclosure.
        // Empty text or successful tools alone never establish this outcome.
        break;
      case 'answered':
        if (item.publicText && item.sourceMessageIds?.some(id => outcome.message_ids.includes(id))) visible.add(item.id);
        break;
      case 'input_required':
        if (item.userInputRequestId === outcome.request_id) visible.add(item.id);
        break;
      case 'waiting':
        if (item.userInputRequestId === outcome.request_id || item.publicText) visible.add(item.id);
        break;
      case 'unavailable':
        if (item.turnOutcome?.kind === 'unavailable') visible.add(item.id);
        break;
      case 'failed':
      case 'cancelled':
      case 'paused':
        // Partial output is also selected by exact committed identities. A
        // late transient row cannot replace the preserved failure result.
        if (item.terminal || (item.publicText && item.sourceMessageIds?.some(id => outcome.message_ids.includes(id)))) visible.add(item.id);
        break;
    }
  }
  return visible;
}

const toProcessReceipt = (entry: TurnDisclosureInputItem): TurnDisclosureOutputItem => ({
  type: 'process_receipt',
  id: `receipt-${entry.id}`,
  itemId: entry.id,
});

export function assignTurnIdsFromUserRequests(
  items: TurnDisclosureInputItem[],
  options: AssignTurnIdOptions = {}
): TurnDisclosureInputItem[] {
  const output: TurnDisclosureInputItem[] = [];
  let currentTurnId: MessageId | undefined;
  let authoritativeTurnId: MessageId | undefined;
  let requestBoundaryTurnId: MessageId | undefined;
  let currentRequestFallbackIndices: number[] = [];
  const retiredTurnIds = new Set<MessageId>();

  for (const entry of items) {
    if (entry.role === 'user') {
      if (entry.inputKind === 'steering') {
        output.push(entry);
        continue;
      }
      if (requestBoundaryTurnId) retiredTurnIds.add(requestBoundaryTurnId);
      if (currentTurnId) retiredTurnIds.add(currentTurnId);
      if (authoritativeTurnId) retiredTurnIds.add(authoritativeTurnId);
      requestBoundaryTurnId = entry.turnId;
      const activeRootTurnId =
        options.activeTurnId &&
        options.activeRequestMessageId &&
        entry.turnId === options.activeRequestMessageId
          ? options.activeTurnId
          : undefined;
      // Once turn.started has correlated the visible request with the backend
      // root, that pair is stronger evidence than positional stream order.
      // In particular, an unseen delayed event from an older turn must not
      // claim the new request merely because it is the first explicit id.
      currentTurnId = activeRootTurnId ?? entry.turnId;
      authoritativeTurnId = entry.inputKind === 'root' ? currentTurnId : activeRootTurnId;
      currentRequestFallbackIndices = [output.length];
      output.push({ ...entry, turnId: currentTurnId });
      continue;
    }

    // New stream/history rows carry the authoritative owning turn. Preserve it
    // even when a delayed event is interleaved after a newer user request. A
    // user's message id is only a provisional request boundary: the backend
    // mint a distinct root turn id for the response. The first non-retired
    // explicit id therefore becomes the fallback for later transient rows that
    // omit turn_id. Known older ids must not move that boundary back.
    if (entry.turnId) {
      if (!authoritativeTurnId && !retiredTurnIds.has(entry.turnId)) {
        authoritativeTurnId = entry.turnId;
        currentTurnId = entry.turnId;
        // Correlate the provisional user/request boundary (and any early
        // transient rows) with the backend-owned response root. This keeps the
        // logical turn contiguous even though the durable user row and root
        // response deliberately have different message IDs.
        for (const index of currentRequestFallbackIndices) {
          output[index] = { ...output[index], turnId: entry.turnId };
        }
      }
      output.push(entry);
      continue;
    }

    if (entry.role === 'other' || !currentTurnId) {
      output.push({ ...entry, turnId: undefined });
      continue;
    }

    currentRequestFallbackIndices.push(output.length);
    output.push({ ...entry, turnId: currentTurnId });
  }

  return output;
}

const getProcessState = (entry: TurnDisclosureInputItem): TurnDisclosureProcessState => {
  if (entry.processState) return entry.processState;
  if (entry.running) return 'running';
  return 'completed';
};

const getEffectiveProcessState = (
  entry: TurnDisclosureInputItem,
  options: { isClosed: boolean }
): TurnDisclosureProcessState => {
  const state = getProcessState(entry);
  if (options.isClosed && state === 'running') {
    return 'completed';
  }
  return state;
};

const getContentTime = (entry: TurnDisclosureInputItem): number =>
  isTurnTimestamp(entry.displayAt) ? entry.displayAt : entry.createdAt;

const getProcessStartAt = (entry: TurnDisclosureInputItem): number => entry.processStartedAt ?? getContentTime(entry);

const getProcessEndAt = (entry: TurnDisclosureInputItem): number => entry.processEndedAt ?? getContentTime(entry);

const buildEmptyRunningDisclosure = (
  turnId: MessageId,
  segment: TurnDisclosureInputItem[],
  turnStartedAt?: number,
  disclosureId = `turn-disclosure-${turnId}`
): TurnDisclosureOutputItem => {
  const startEntry = segment.findLast((entry) => entry.role === 'user') ?? segment[0];
  const startAt = turnStartedAt ?? (startEntry ? getProcessStartAt(startEntry) : 0);
  const endAt = segment.length ? Math.max(...segment.map(getProcessEndAt)) : startAt;

  return {
    type: 'turn_disclosure',
    id: disclosureId,
    turnId,
    processItemIds: [],
    sourceMessageIds: [],
    startAt,
    endAt,
    state: 'running',
    processItemStates: {},
    running: true,
    defaultCollapsed: false,
  };
};

const buildEmptyRunningSegmentOutput = (
  segment: TurnDisclosureInputItem[],
  disclosure: TurnDisclosureOutputItem
): TurnDisclosureOutputItem[] => {
  const output: TurnDisclosureOutputItem[] = [];
  let insertedDisclosure = false;

  segment.forEach((entry) => {
    if (entry.role === 'metadata') return;
    if (!insertedDisclosure && entry.role !== 'user' && entry.role !== 'other') {
      output.push(disclosure);
      insertedDisclosure = true;
    }
    output.push({ type: 'item', id: entry.id });
  });

  if (!insertedDisclosure) {
    output.push(disclosure);
  }

  return output;
};

const buildEmptyClosedSegmentOutput = (
  turnId: MessageId,
  segment: TurnDisclosureInputItem[],
  resultAnchor: TurnDisclosureInputItem | undefined,
  turnStartedAt: number,
  turnEndedAt: number,
  disclosureId = `turn-disclosure-${turnId}`
): TurnDisclosureOutputItem[] => {
  const state = resultAnchor?.processState === 'failed'
    ? 'failed'
    : resultAnchor?.processState === 'canceled'
      ? 'canceled'
      : 'completed';
  const disclosure: TurnDisclosureOutputItem = {
    type: 'turn_disclosure',
    id: disclosureId,
    turnId,
    processItemIds: [],
    sourceMessageIds: [],
    startAt: turnStartedAt,
    endAt: turnEndedAt,
    state,
    processItemStates: {},
    running: false,
    defaultCollapsed: true,
  };
  const output: TurnDisclosureOutputItem[] = [];
  let inserted = false;
  for (const entry of segment) {
    if (entry.role === 'metadata') continue;
    if (entry === resultAnchor && !inserted) {
      output.push(disclosure);
      inserted = true;
    }
    output.push({ type: 'item', id: entry.id });
  }
  if (!inserted) output.push(disclosure);
  return output;
};

function buildSegmentOutput(
  segment: TurnDisclosureInputItem[],
  isClosed: boolean,
  resultAnchor?: TurnDisclosureInputItem,
  turnStartedAt?: number,
  turnEndedAt?: number,
  publicResultIds: ReadonlySet<string> = new Set(),
  hasProcessRecords = false,
  disclosureId = `turn-disclosure-${segment[0]?.turnId}`
): TurnDisclosureOutputItem[] {
  const turnId = segment[0]?.turnId;
  if (!turnId) return segment.map((entry) => ({ type: 'item', id: entry.id }));

  // A result receipt may precede its message rows in history. Membership is
  // exact and independent of arrival order, timestamps, or an empty tail row.
  const isVisibleAssistant = (index: number) => publicResultIds.has(segment[index].id)
    && (isClosed || Boolean(segment[index].userInputRequestId));
  const stateOptions = { isClosed };

  const processItems = segment.filter((entry, index) => {
    if (entry.role === 'user' || entry.role === 'metadata' || entry.role === 'other') return false;
    return !isVisibleAssistant(index);
  });

  if (!processItems.length) {
    if (!isClosed) {
      return buildEmptyRunningSegmentOutput(segment, buildEmptyRunningDisclosure(turnId, segment, turnStartedAt, disclosureId));
    }
    const visibleEntries = segment.filter((entry) => entry.role !== 'metadata');
    const fallbackStart = visibleEntries.length
      ? Math.min(...visibleEntries.map(getProcessStartAt))
      : 0;
    const fallbackEnd = resultAnchor
      ? getProcessEndAt(resultAnchor)
      : visibleEntries.length
        ? Math.max(...visibleEntries.map(getProcessEndAt))
        : fallbackStart;
    if (resultAnchor || turnEndedAt !== undefined) {
      return buildEmptyClosedSegmentOutput(
        turnId,
        segment,
        resultAnchor,
        turnStartedAt ?? fallbackStart,
        turnEndedAt ?? fallbackEnd,
        disclosureId
      );
    }
    if (hasProcessRecords) return buildEmptyClosedSegmentOutput(turnId, segment, resultAnchor,
      turnStartedAt ?? Number.NaN, turnEndedAt ?? Number.NaN, disclosureId);
    return visibleEntries.map((entry) => ({ type: 'item', id: entry.id }));
  }

  const terminalProcessState = getEffectiveProcessState(processItems.at(-1)!, stateOptions);
  const terminalState = resultAnchor
    ? getEffectiveProcessState(resultAnchor, stateOptions)
    : terminalProcessState;
  // Intermediate failures remain visible in the expanded trace, while the
  // terminal answer/error owns the settled Turn outcome.
  const state: TurnDisclosureProcessState = isClosed
    ? terminalState === 'failed' || terminalState === 'canceled'
      ? terminalState
      : 'completed'
    : 'running';

  const disclosure: TurnDisclosureOutputItem = {
    type: 'turn_disclosure',
    id: disclosureId,
    turnId,
    processItemIds: processItems.map((entry) => entry.id),
    sourceMessageIds: unique(processItems.flatMap((entry) => entry.sourceMessageIds ?? [])),
    // The header measures the whole task, not individual process rows. Keep the
    // same request timestamp while live and after completion so adding the first
    // thinking/tool row can never restart the timer.
    startAt: turnStartedAt ?? Math.min(...processItems.map(getProcessStartAt)),
    // A final answer is the authoritative task boundary. Process intervals are
    // only a fallback for background/process-only turns with no final answer.
    endAt: turnEndedAt ?? (resultAnchor
      ? getProcessEndAt(resultAnchor)
      : Math.max(...processItems.map(getProcessEndAt))),
    state,
    processItemStates: Object.fromEntries(
      processItems.map((entry) => [entry.id, getEffectiveProcessState(entry, stateOptions)])
    ),
    running: state === 'running',
    defaultCollapsed: state !== 'running',
  };

  const output: TurnDisclosureOutputItem[] = [];
  let insertedDisclosure = false;

  segment.forEach((entry, index) => {
    if (entry.role === 'metadata') return;
    if (entry.role !== 'user' && entry.role !== 'other' && !isVisibleAssistant(index)) {
      return;
    }

    if (isVisibleAssistant(index) && !insertedDisclosure) {
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

const getDisclosureStateObservedAt = (
  disclosure: Extract<TurnDisclosureOutputItem, { type: 'turn_disclosure' }>,
  processObservedAtByItemId: ReadonlyMap<string, number>
): number => {
  if (!disclosure.processItemIds.length) return disclosure.endAt;
  const observedTimes = disclosure.processItemIds
    .map((itemId) => processObservedAtByItemId.get(itemId))
    .filter((value): value is number => value !== undefined);
  return observedTimes.length ? Math.max(...observedTimes) : disclosure.endAt;
};

const coalesceProcessBoundaries = (
  items: TurnDisclosureOutputItem[],
  processObservedAtByItemId: ReadonlyMap<string, number>
): TurnDisclosureOutputItem[] => {
  const output: TurnDisclosureOutputItem[] = [];
  const disclosureIndexById = new Map<string, number>();

  for (const item of items) {
    if (item.type !== 'turn_disclosure') {
      output.push(item);
      continue;
    }

    const existingIndex = disclosureIndexById.get(item.id);
    if (existingIndex === undefined) {
      disclosureIndexById.set(item.id, output.length);
      output.push(item);
      continue;
    }

    const existing = output[existingIndex];
    if (existing.type !== 'turn_disclosure') continue;
    // Header state describes the latest process observation, not the most
    // severe state ever seen. Duration endAt also includes the global final
    // answer, so it cannot safely decide freshness between split fragments.
    // On equal process timestamps, the later transcript fragment wins.
    const existingStateObservedAt = getDisclosureStateObservedAt(existing, processObservedAtByItemId);
    const itemStateObservedAt = getDisclosureStateObservedAt(item, processObservedAtByItemId);
    const state = itemStateObservedAt >= existingStateObservedAt ? item.state : existing.state;
    output[existingIndex] = {
      ...existing,
      processItemIds: unique([...existing.processItemIds, ...item.processItemIds]),
      sourceMessageIds: unique([...existing.sourceMessageIds, ...item.sourceMessageIds]),
      startAt: Math.min(existing.startAt, item.startAt),
      endAt: Math.max(existing.endAt, item.endAt),
      state,
      processItemStates: { ...existing.processItemStates, ...item.processItemStates },
      running: state === 'running',
      defaultCollapsed: state !== 'running',
    };
  }

  return output;
};

const placeProcessBoundaries = (
  items: TurnDisclosureOutputItem[],
  requestByBoundary: ReadonlyMap<string, TurnDisclosureInputItem>,
  resultAnchorByBoundary: ReadonlyMap<string, TurnDisclosureInputItem>
): TurnDisclosureOutputItem[] => {
  const disclosureByBoundary = new Map<string, Extract<TurnDisclosureOutputItem, { type: 'turn_disclosure' }>>();
  for (const item of items) {
    if (item.type === 'turn_disclosure') disclosureByBoundary.set(item.id, item);
  }

  const visibleItemIds = new Set(items.flatMap((item) => (item.type === 'item' ? [item.id] : [])));
  const requestBoundaryByItemId = new Map<string, string>();
  for (const [boundary, request] of requestByBoundary) {
    if (visibleItemIds.has(request.id)) requestBoundaryByItemId.set(request.id, boundary);
  }
  const requestAnchoredBoundaries = new Set(requestBoundaryByItemId.values());

  const resultBoundaryByItemId = new Map<string, string>();
  for (const [boundary, resultAnchor] of resultAnchorByBoundary) {
    if (!requestAnchoredBoundaries.has(boundary) && visibleItemIds.has(resultAnchor.id)) {
      resultBoundaryByItemId.set(resultAnchor.id, boundary);
    }
  }
  const anchoredBoundaries = new Set([...requestAnchoredBoundaries, ...resultBoundaryByItemId.values()]);

  const output: TurnDisclosureOutputItem[] = [];
  for (const item of items) {
    if (item.type === 'turn_disclosure' && anchoredBoundaries.has(item.id)) {
      continue;
    }
    if (item.type === 'item') {
      const boundary = resultBoundaryByItemId.get(item.id);
      const disclosure = boundary ? disclosureByBoundary.get(boundary) : undefined;
      if (disclosure) output.push(disclosure);
    }
    output.push(item);
    if (item.type === 'item') {
      const boundary = requestBoundaryByItemId.get(item.id);
      const disclosure = boundary ? disclosureByBoundary.get(boundary) : undefined;
      if (disclosure) output.push(disclosure);
    }
  }
  return output;
};

const applyStopNotice = (
  items: TurnDisclosureOutputItem[],
  stopNotice?: { stoppedAt: number }
): TurnDisclosureOutputItem[] => {
  if (!stopNotice) return items;
  const lastIndex = items.findLastIndex((entry) => entry.type === 'turn_disclosure');
  if (lastIndex === -1) return items;
  const disclosure = items[lastIndex];
  if (disclosure.type !== 'turn_disclosure') return items;
  // A live turn must keep ticking: the optimistic stop path flips
  // isProcessing first, so a running disclosure here means the notice is
  // stale (e.g. stop failed and the turn resumed).
  if (disclosure.running) return items;
  // A notice from an earlier turn must not cancel a newer one.
  if (stopNotice.stoppedAt < disclosure.startAt) return items;
  const next = items.slice();
  next[lastIndex] = {
    ...disclosure,
    state: 'canceled',
    endAt: stopNotice.stoppedAt,
    running: false,
    defaultCollapsed: true,
  };
  return next;
};

export function buildTurnDisclosureItems(
  items: TurnDisclosureInputItem[],
  options: BuildTurnDisclosureOptions = {}
): TurnDisclosureOutputItem[] {
  const output: TurnDisclosureOutputItem[] = [];
  let segment: TurnDisclosureInputItem[] = [];
  const activeTurnId = options.tailClosed === true ? undefined : options.activeTurnId;
  const requestByBoundary = new Map<string, TurnDisclosureInputItem>();
  const resultAnchorByBoundary = new Map<string, TurnDisclosureInputItem>();
  const turnStartedAtByTurn = new Map<MessageId, number>();
  const turnEndedAtByTurn = new Map<MessageId, number>();
  const terminalStateByTurn = new Map<MessageId, TurnDisclosureProcessState>();
  const authoritativeStartedTurns = new Set<MessageId>();
  const processObservedAtByItemId = new Map<string, number>();
  const indexByItem = new Map(items.map((item, index) => [item, index]));
  const comparePosition = (left: TurnDisclosureInputItem, right: TurnDisclosureInputItem) =>
    left.sequence !== undefined && right.sequence !== undefined ? left.sequence - right.sequence
      : left.sequence === undefined && right.sequence === undefined ? left.createdAt - right.createdAt
        : indexByItem.get(left)! - indexByItem.get(right)!;
  const firstProcessByTurn = new Map<MessageId, TurnDisclosureInputItem>();
  for (const item of items) {
    if (!item.turnId || item.role === 'user' || item.role === 'metadata' || item.role === 'other') continue;
    const previous = firstProcessByTurn.get(item.turnId);
    if (!previous || comparePosition(item, previous) < 0) firstProcessByTurn.set(item.turnId, item);
  }
  const steeringByTurn = new Map<MessageId, TurnDisclosureInputItem[]>();
  for (const item of items) {
    if (!item.turnId || item.inputKind !== 'steering') continue;
    // Opening inputs share the initial process. An actual intervention after
    // process evidence creates a durable reading boundary within the same Turn.
    const firstProcess = firstProcessByTurn.get(item.turnId);
    if (!firstProcess || comparePosition(firstProcess, item) >= 0) continue;
    const boundaries = steeringByTurn.get(item.turnId) ?? [];
    boundaries.push(item);
    steeringByTurn.set(item.turnId, boundaries);
  }
  for (const boundaries of steeringByTurn.values()) boundaries.sort(comparePosition);
  const boundaryOf = (item: TurnDisclosureInputItem): string => {
    const intervention = item.turnId && steeringByTurn.get(item.turnId)?.findLast(candidate => comparePosition(candidate, item) <= 0);
    return `turn-disclosure-${item.turnId}${intervention ? `:${intervention.id}` : ''}`;
  };

  if (activeTurnId && isTurnTimestamp(options.activeTurnStartedAt)) {
    turnStartedAtByTurn.set(activeTurnId, options.activeTurnStartedAt);
    authoritativeStartedTurns.add(activeTurnId);
  }

  for (const item of items) {
    if (item.turnId && isTurnTimestamp(item.turnStartedAt)) {
      turnStartedAtByTurn.set(item.turnId, item.turnStartedAt);
      authoritativeStartedTurns.add(item.turnId);
    }
    if (item.turnId && isTurnTimestamp(item.turnEndedAt)) {
      turnEndedAtByTurn.set(item.turnId, item.turnEndedAt);
      if (item.role === 'metadata' && item.processState && item.processState !== 'running') {
        terminalStateByTurn.set(item.turnId, item.processState);
      }
    }
    if (item.turnId && item.role === 'user') {
      const boundary = boundaryOf(item);
      if (!requestByBoundary.has(boundary) || item.inputKind === 'steering') requestByBoundary.set(boundary, item);
      const currentStart = turnStartedAtByTurn.get(item.turnId);
      const requestStartedAt = getProcessStartAt(item);
      if (
        !authoritativeStartedTurns.has(item.turnId) &&
        (currentStart === undefined || requestStartedAt < currentStart)
      ) {
        // Before turn.started arrives, use the accepted request's wall clock,
        // never its canonical history-order cursor. Exact Turn timing wins later.
        turnStartedAtByTurn.set(item.turnId, requestStartedAt);
      }
    }
    if (item.role !== 'user' && item.role !== 'metadata' && item.role !== 'other') {
      processObservedAtByItemId.set(item.id, getProcessEndAt(item));
    }
  }

  const publicResultIds = selectTurnPublicResultIds(items);
  // This anchor only places the disclosure. It never decides which content is
  // public: every public member is already selected by the exact receipt.
  for (const item of items) {
    if (item.turnId && publicResultIds.has(item.id)) {
      const boundary = boundaryOf(item);
      const current = resultAnchorByBoundary.get(boundary);
      if (!current?.terminal || item.terminal) resultAnchorByBoundary.set(boundary, item);
    }
  }
  const flush = (fallbackClosed: boolean) => {
    if (!segment.length) return;
    const segmentTurnId = segment[0]?.turnId;
    const boundary = boundaryOf(segment[0]);
    const boundaries = segmentTurnId ? steeringByTurn.get(segmentTurnId) : undefined;
    const lastBoundary = boundaries?.at(-1);
    const isEarlierBoundary = Boolean(lastBoundary && boundary !== boundaryOf(lastBoundary));
    const isClosed = (segmentTurnId !== undefined && turnEndedAtByTurn.has(segmentTurnId))
      || isEarlierBoundary || options.tailClosed === true || (activeTurnId ? segmentTurnId !== activeTurnId : fallbackClosed);
    output.push(
      ...buildSegmentOutput(
        segment,
        isClosed,
        resultAnchorByBoundary.get(boundary),
        segmentTurnId ? turnStartedAtByTurn.get(segmentTurnId) : undefined,
        !isEarlierBoundary && segmentTurnId ? turnEndedAtByTurn.get(segmentTurnId) : undefined,
        publicResultIds,
        segmentTurnId !== undefined && options.processRecordTurnIds?.has(segmentTurnId),
        boundary
      ).map(entry => entry.type === 'turn_disclosure' && isEarlierBoundary ? { ...entry, earlierSegment: true } : entry)
    );
    segment = [];
  };

  for (const item of items) {
    if (!item.turnId) {
      flush(true);
      if (item.role !== 'metadata') {
        output.push(item.role === 'process' ? toProcessReceipt(item) : { type: 'item', id: item.id });
      }
      continue;
    }

    const currentTurnId = segment[0]?.turnId;
    if (currentTurnId && (currentTurnId !== item.turnId || boundaryOf(segment[0]) !== boundaryOf(item))) {
      flush(true);
    }

    segment.push(item);
  }

  flush(options.tailClosed === true);
  // Delayed rows join their original process boundary. An intervention keeps
  // separate disclosures before and after it, even though both own one Turn.
  const coalesced = coalesceProcessBoundaries(output, processObservedAtByItemId);
  const placed = placeProcessBoundaries(coalesced, requestByBoundary, resultAnchorByBoundary);
  // Durable terminal metadata wins over partial text and stale live tool rows,
  // including after reload when the in-memory stop notice no longer exists.
  const authoritative = placed.map((item): TurnDisclosureOutputItem => {
    if (item.type !== 'turn_disclosure') return item;
    const lastIntervention = steeringByTurn.get(item.turnId)?.at(-1);
    if (lastIntervention && item.id !== boundaryOf(lastIntervention)) return item;
    const state = terminalStateByTurn.get(item.turnId);
    return state ? { ...item, state, running: false, defaultCollapsed: true } : item;
  });
  return applyStopNotice(authoritative, options.stopNotice);
}
