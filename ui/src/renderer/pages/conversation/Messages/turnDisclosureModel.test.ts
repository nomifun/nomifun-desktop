/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import {
  assignTurnIdsFromUserRequests,
  buildTurnDisclosureItems,
  selectTurnPublicResultIds,
  type TurnDisclosureInputItem,
} from './turnDisclosureModel';
import { parseMessageId } from '@/common/types/ids';

const TURN_1 = parseMessageId('0190f5fe-7c00-7a00-8000-000000000001');
const TURN_2 = parseMessageId('0190f5fe-7c00-7a00-8000-000000000002');
const AGENT_ROOT_1 = parseMessageId('0190f5fe-7c00-7a00-8000-000000000011');
const SOURCE_1 = parseMessageId('0190f5fe-7c00-7a00-8000-000000000021');
const SOURCE_2 = parseMessageId('0190f5fe-7c00-7a00-8000-000000000022');
const DISCLOSURE_1 = `turn-disclosure-${TURN_1}`;
const DISCLOSURE_2 = `turn-disclosure-${TURN_2}`;

const item = (
  id: string,
  role: TurnDisclosureInputItem['role'],
  options: Partial<TurnDisclosureInputItem> = {}
): TurnDisclosureInputItem => ({
  id,
  turnId: TURN_1,
  role,
  createdAt: options.createdAt ?? 1000,
  sourceMessageIds: options.sourceMessageIds ?? [],
  standalonePublic: role === 'assistant',
  ...options,
});

describe('buildTurnDisclosureItems', () => {
  test('a live waiting question stays outside the process disclosure before its Turn ends', () => {
    const question = item('waiting-question', 'assistant', { standalonePublic: false, userInputRequestId: 'authorize',
      turnOutcome: { kind: 'waiting', wait_seq: 41, request_id: 'authorize', reason: 'user_input' } });
    const result = buildTurnDisclosureItems([item('user', 'user'), item('process', 'process'), question],
      { tailClosed: false, activeTurnId: TURN_1 });
    expect(result.some(entry => entry.type === 'item' && entry.id === 'waiting-question')).toBe(true);
    const disclosure = result.find(entry => entry.type === 'turn_disclosure');
    expect(disclosure?.type === 'turn_disclosure' && disclosure.processItemIds.includes('waiting-question')).toBe(false);
  });
  test('a closed canonical Turn without a result receipt does not elect any text by position or time', () => {
    const entries = [item('user', 'user'),
      item('first', 'assistant', { publicText: true, standalonePublic: false, createdAt: 2000 }),
      item('last', 'assistant', { publicText: true, standalonePublic: false, createdAt: 9000 })];
    const visible = buildTurnDisclosureItems(entries, { tailClosed: true }).filter(entry => entry.type === 'item');
    expect(visible.map(entry => entry.id)).toEqual(['user']);
    expect([...selectTurnPublicResultIds(entries)]).toEqual([]);
  });

  test.each([false, true])('accepted input uses its wall clock before canonical start, with process=%p', (withProcess) => {
    const acceptedAt = 4_000_000;
    const entries = [
      item('user', 'user', { createdAt: 1_000, displayAt: acceptedAt }),
      ...(withProcess ? [item('phase', 'process_content', { createdAt: 1_001, processState: 'running' })] : []),
    ];
    const disclosure = buildTurnDisclosureItems(entries, { tailClosed: false })
      .find(entry => entry.type === 'turn_disclosure');
    if (disclosure?.type !== 'turn_disclosure') throw new Error('active disclosure missing');
    expect(disclosure.startAt).toBe(acceptedAt);
    expect(disclosure.running).toBe(true);

    const started = buildTurnDisclosureItems(entries, {
      activeTurnId: TURN_1, activeTurnStartedAt: acceptedAt + 100,
    }).find(entry => entry.type === 'turn_disclosure');
    if (started?.type !== 'turn_disclosure') throw new Error('started disclosure missing');
    expect(started.startAt).toBe(acceptedAt + 100);
    expect(started.id).toBe(disclosure.id);
  });

  test('canonical history owns the interval regardless of input wall time or row order', () => {
    const metadata = item('summary', 'metadata', {
      createdAt: 1_001, turnStartedAt: 4_000_100, turnEndedAt: 4_003_100,
    });
    const user = item('user', 'user', { createdAt: 1_000, displayAt: 4_000_000 });
    for (const entries of [[metadata, user], [user, metadata]]) {
      const disclosure = buildTurnDisclosureItems(entries, { tailClosed: true })
        .find(entry => entry.type === 'turn_disclosure');
      if (disclosure?.type !== 'turn_disclosure') throw new Error('settled disclosure missing');
      expect(disclosure.startAt).toBe(4_000_100);
      expect(disclosure.endAt).toBe(4_003_100);
      expect(disclosure.running).toBe(false);
    }
  });

  test.each([false, true])('text-only fallback keeps start and end on the same clock, display time=%p', (withDisplayTime) => {
    const startAt = withDisplayTime ? 4_000_000 : 1_000;
    const endAt = withDisplayTime ? 4_003_000 : 4_000;
    const disclosure = buildTurnDisclosureItems([
      item('user', 'user', { createdAt: 1_000, ...(withDisplayTime ? { displayAt: startAt } : {}) }),
      item('final', 'assistant', { createdAt: 4_000, ...(withDisplayTime ? { displayAt: endAt } : {}) }),
    ], { tailClosed: true }).find(entry => entry.type === 'turn_disclosure');
    if (disclosure?.type !== 'turn_disclosure') throw new Error('text-only disclosure missing');
    expect(disclosure.startAt).toBe(startAt);
    expect(disclosure.endAt).toBe(endAt);
    expect(disclosure.endAt - disclosure.startAt).toBe(3_000);
  });

  test.each([null, Number.NaN, Number.POSITIVE_INFINITY, '3000'])('ignores invalid canonical finish %p on a running Turn', (finish) => {
    const result = buildTurnDisclosureItems([
      item('summary', 'metadata', {
        turnId: TURN_1,
        createdAt: 1000,
        turnStartedAt: 2000,
        turnEndedAt: finish as unknown as number,
        processState: 'running',
      }),
      item('phase', 'process_content', { turnId: TURN_1, createdAt: 3000, processState: 'running' }),
    ], { activeTurnId: TURN_1 });
    const disclosure = result.find(entry => entry.type === 'turn_disclosure');
    expect(disclosure?.type).toBe('turn_disclosure');
    if (disclosure?.type !== 'turn_disclosure') return;
    expect(disclosure.startAt).toBe(2000);
    expect(disclosure.running).toBe(true);
    expect(disclosure.defaultCollapsed).toBe(false);
    expect(disclosure.processItemStates.phase).toBe('running');
    expect(Number.isFinite(disclosure.endAt)).toBe(true);
  });

  test('restores the exact active Turn clock when its start is outside the history window', () => {
    const result = buildTurnDisclosureItems([
      item('phase', 'process_content', { turnId: TURN_1, createdAt: 8000, processState: 'running' }),
      item('old-phase', 'process_content', { turnId: TURN_2, createdAt: 2000, processState: 'completed' }),
    ], { activeTurnId: TURN_1, activeTurnStartedAt: 1000 });
    const active = result.find(entry => entry.type === 'turn_disclosure' && entry.turnId === TURN_1);
    expect(active?.type).toBe('turn_disclosure');
    if (active?.type !== 'turn_disclosure') return;
    expect(active.startAt).toBe(1000);
    expect(active.running).toBe(true);
    const old = result.find(entry => entry.type === 'turn_disclosure' && entry.turnId === TURN_2);
    if (old?.type !== 'turn_disclosure') throw new Error('old Turn disclosure missing');
    expect(old.startAt).toBe(2000);
    expect(old.running).toBe(false);
  });

  test('an answered receipt keeps every exact public part outside the process disclosure', () => {
    const first=item('first','assistant',{createdAt:2000,sourceMessageIds:[SOURCE_1],publicText:true});
    const tail=item('tail','assistant',{createdAt:3000,sourceMessageIds:[SOURCE_2],publicText:true});
    const summary=item('summary','metadata',{turnOutcome:{kind:'answered',message_ids:[SOURCE_1,SOURCE_2]}});
    const result=buildTurnDisclosureItems([item('user','user'),summary,first,tail],{tailClosed:true});
    expect(result.filter(entry=>entry.type==='item').map(entry=>entry.id)).toEqual(['user','first','tail']);
    expect([...selectTurnPublicResultIds([summary,first,tail])]).toEqual(['first','tail']);
  });

  test('an explicit silent result keeps the completed tool trace and clock without electing a reply', () => {
    const entries = [item('user', 'user'),
      item('tool', 'process', { createdAt: 2000, processState: 'completed', sourceMessageIds: [SOURCE_1] }),
      item('process-text', 'assistant', { createdAt: 2500, publicText: true, sourceMessageIds: [SOURCE_2] }),
      item('summary', 'metadata', { createdAt: 3000, turnStartedAt: 1000, turnEndedAt: 3000,
        processState: 'completed', turnOutcome: { kind: 'silent' } })];
    const result = buildTurnDisclosureItems(entries, { tailClosed: true });
    expect([...selectTurnPublicResultIds(entries)]).toEqual([]);
    expect(result.filter(entry => entry.type === 'item').map(entry => entry.id)).toEqual(['user']);
    expect(result.find(entry => entry.type === 'turn_disclosure')).toMatchObject({
      state: 'completed', running: false, defaultCollapsed: true, startAt: 1000, endAt: 3000,
      processItemIds: ['tool', 'process-text'],
    });
  });

  test('a silent result has no unavailable placeholder and cannot hide the following answered Turn', () => {
    const entries = [item('user', 'user'),
      item('silent', 'metadata', { turnOutcome: { kind: 'silent' }, turnStartedAt: 1000, turnEndedAt: 2000 }),
      item('next-user', 'user', { turnId: TURN_2, createdAt: 3000 }),
      item('next-summary', 'metadata', { turnId: TURN_2, turnStartedAt: 3000, turnEndedAt: 4000,
        turnOutcome: { kind: 'answered', message_ids: [SOURCE_2] } }),
      item('answer', 'assistant', { turnId: TURN_2, createdAt: 4000, publicText: true, sourceMessageIds: [SOURCE_2] })];
    expect([...selectTurnPublicResultIds(entries)]).toEqual(['answer']);
    const result = buildTurnDisclosureItems(entries, { tailClosed: true });
    expect(result.filter(entry => entry.type === 'item').map(entry => entry.id)).toEqual(['user', 'next-user', 'answer']);
    expect(result.find(entry => entry.type === 'turn_disclosure' && entry.turnId === TURN_1)).toMatchObject({
      state: 'completed', running: false, startAt: 1000, endAt: 2000,
    });
  });

  test('unreferenced and missing public sources cannot become the final reply', () => {
    const first=item('preamble','assistant',{createdAt:2000,sourceMessageIds:[SOURCE_1],publicText:true});
    const final=item('final','assistant',{createdAt:4000,sourceMessageIds:[SOURCE_2],publicText:true});
    const summary=item('summary','metadata',{turnOutcome:{kind:'answered',message_ids:[SOURCE_2]}});
    const result=buildTurnDisclosureItems([item('user','user'),summary,first,item('tool','process',{createdAt:3000}),final],{tailClosed:true});
    expect(result.filter(entry=>entry.type==='item').map(entry=>entry.id)).toEqual(['user','final']);
    expect([...selectTurnPublicResultIds([summary,first])]).toEqual([]);
  });

  test('a durable cancelled summary survives reload and partial assistant text', () => {
    for (const withProcess of [false, true]) {
      const result = buildTurnDisclosureItems([
        item('user', 'user'),
        ...(withProcess ? [item('tool','process',{createdAt:2000,processState:'canceled'})] : []),
        item('partial','assistant',{createdAt:2500,processState:'completed'}),
        item('summary','metadata',{createdAt:3000,turnEndedAt:3000,processState:'canceled'}),
      ], {tailClosed:true});
      const disclosure = result.find(entry=>entry.type==='turn_disclosure');
      expect(disclosure?.state).toBe('canceled');
      expect(disclosure?.running).toBe(false);
      expect(disclosure?.endAt).toBe(3000);
    }
  });

  test('collapses completed intermediate steps while keeping the final answer visible', () => {
    const result = buildTurnDisclosureItems(
      [
        item('user', 'user', { createdAt: 1000 }),
        item('analysis', 'process', { createdAt: 2000, sourceMessageIds: [SOURCE_1] }),
        item('tool', 'process', { createdAt: 3000, sourceMessageIds: [SOURCE_2] }),
        item('final', 'assistant', { createdAt: 5000 }),
      ],
      { tailClosed: true }
    );

    expect(result.map((entry) => entry.type === 'item' ? entry.id : entry.id)).toEqual([
      'user',
      DISCLOSURE_1,
      'final',
    ]);

    const disclosure = result[1];
    expect(disclosure.type).toBe('turn_disclosure');
    if (disclosure.type !== 'turn_disclosure') return;
    expect(disclosure.defaultCollapsed).toBe(true);
    expect(disclosure.state).toBe('completed');
    expect(disclosure.processItemIds).toEqual(['analysis', 'tool']);
    expect(disclosure.startAt).toBe(1000);
    expect(disclosure.endAt).toBe(5000);
    expect(disclosure.sourceMessageIds).toEqual([SOURCE_1, SOURCE_2]);
  });

  test('anchors the duration to the request and final answer timestamps', () => {
    const result = buildTurnDisclosureItems(
      [
        item('user', 'user', { createdAt: 0 }),
        item('analysis', 'process', {
          createdAt: 35000,
          processStartedAt: 1000,
          processEndedAt: 35000,
        }),
        item('tool', 'process', { createdAt: 33000 }),
        item('final', 'assistant', { createdAt: 35600 }),
      ],
      { tailClosed: true }
    );

    const disclosure = result[1];
    expect(disclosure.type).toBe('turn_disclosure');
    if (disclosure.type !== 'turn_disclosure') return;
    expect(disclosure.processItemIds).toEqual(['analysis', 'tool']);
    expect(disclosure.startAt).toBe(0);
    expect(disclosure.endAt).toBe(35600);
  });

  test('uses canonical turn timing instead of synthetic message timestamps', () => {
    const result = buildTurnDisclosureItems(
      [
        item('user', 'user', { createdAt: 1_000 }),
        item('turn-metadata', 'metadata', {
          createdAt: 1_001,
          turnStartedAt: 4_000_000,
          turnEndedAt: 4_002_000,
        }),
        item('error', 'assistant', {
          createdAt: 4_080_000,
          processState: 'failed',
          terminal: true,
        }),
      ],
      { tailClosed: true }
    );

    expect(result.map((entry) => entry.id)).toEqual(['user', DISCLOSURE_1, 'error']);
    const disclosure = result[1];
    expect(disclosure.type).toBe('turn_disclosure');
    if (disclosure.type !== 'turn_disclosure') return;
    expect(disclosure.startAt).toBe(4_000_000);
    expect(disclosure.endAt).toBe(4_002_000);
    expect(disclosure.processItemIds).toEqual([]);
    expect(disclosure.state).toBe('failed');
  });

  test('keeps a terminal error outside the process when partial text arrived later', () => {
    const result = buildTurnDisclosureItems(
      [
        item('user', 'user', { createdAt: 1000 }),
        item('terminal-error', 'assistant', {
          createdAt: 1500,
          processState: 'failed',
          terminal: true, turnOutcome: { kind: 'failed', message_ids: [] },
        }),
        item('late-partial-text', 'assistant', { createdAt: 2000 }),
      ],
      { tailClosed: true }
    );

    expect(result.map((entry) => entry.id)).toEqual(['user', DISCLOSURE_1, 'terminal-error']);
    const disclosure = result[1];
    expect(disclosure.type).toBe('turn_disclosure');
    if (disclosure.type !== 'turn_disclosure') return;
    expect(disclosure.processItemIds).toEqual(['late-partial-text']);
    expect(disclosure.state).toBe('failed');
  });

  test('keeps the last partial result visible beside a terminal failure', () => {
    const result = buildTurnDisclosureItems([
      item('user', 'user', { createdAt: 1000 }),
      item('progress-note', 'assistant', { createdAt: 1500 }),
      item('patch', 'process', { createdAt: 2000, processState: 'failed' }),
      item('partial-result', 'assistant', { publicText: true, sourceMessageIds: [SOURCE_1], createdAt: 2500 }),
      item('terminal-error', 'assistant', { createdAt: 3000, processState: 'failed', terminal: true, turnOutcome: { kind: 'failed', message_ids: [SOURCE_1] } }),
    ], { tailClosed: true });
    expect(result.map(entry => entry.id)).toEqual(['user', DISCLOSURE_1, 'partial-result', 'terminal-error']);
    const disclosure = result.find(entry => entry.type === 'turn_disclosure');
    expect(disclosure?.type).toBe('turn_disclosure');
    if (disclosure?.type !== 'turn_disclosure') throw new Error('missing disclosure');
    expect(disclosure.processItemIds).toEqual(['progress-note', 'patch']);
    expect(disclosure.state).toBe('failed');
    expect(disclosure.endAt).toBe(3000);
  });

  test('preserves exact partial-result membership independently of message wall time', () => {
    const result = buildTurnDisclosureItems([
      item('user', 'user', { createdAt: 1000 }),
      item('terminal-error', 'assistant', { createdAt: 1020, turnEndedAt: 4000, processState: 'failed', terminal: true, turnOutcome: { kind: 'failed', message_ids: [SOURCE_1] } }),
      item('partial-result', 'assistant', { publicText: true, sourceMessageIds: [SOURCE_1], createdAt: 1050, displayAt: 3500 }),
      item('late-text', 'assistant', { createdAt: 1060, displayAt: 4500 }),
      item('delayed-old-text', 'assistant', { createdAt: 1070, displayAt: 3000 }),
    ], { tailClosed: true });
    const visible = result.filter(entry => entry.type === 'item').map(entry => entry.id);
    expect(visible).toEqual(['user', 'terminal-error', 'partial-result']);
    const disclosure = result.find(entry => entry.type === 'turn_disclosure');
    if (disclosure?.type !== 'turn_disclosure') throw new Error('missing disclosure');
    expect(disclosure.processItemIds).toEqual(['late-text', 'delayed-old-text']);
    expect(disclosure.state).toBe('failed');
    expect(disclosure.endAt).toBe(4000);
  });

  test('late text and another turn cannot replace the preserved failure result', () => {
    const result = buildTurnDisclosureItems([
      item('user', 'user', { createdAt: 1000 }),
      item('partial-result', 'assistant', { publicText: true, sourceMessageIds: [SOURCE_1], createdAt: 2500 }),
      item('other-user', 'user', { turnId: TURN_2, createdAt: 2600 }),
      item('other-answer', 'assistant', { turnId: TURN_2, createdAt: 2700 }),
      item('terminal-error', 'assistant', { createdAt: 3000, processState: 'failed', terminal: true, turnOutcome: { kind: 'failed', message_ids: [SOURCE_1] } }),
      item('late-partial', 'assistant', { createdAt: 3500 }),
    ], { tailClosed: true });
    const visible = result.filter(entry => entry.type === 'item').map(entry => entry.id);
    expect(visible).toEqual(['user', 'partial-result', 'other-user', 'other-answer', 'terminal-error']);
    const disclosure = result.find(entry => entry.type === 'turn_disclosure' && entry.turnId === TURN_1);
    if (disclosure?.type !== 'turn_disclosure') throw new Error('missing disclosure');
    expect(disclosure.processItemIds).toEqual(['late-partial']);
    expect(disclosure.state).toBe('failed');
  });

  test('keeps the final assistant answer outside the disclosure when earlier assistant text was intermediate', () => {
    const result = buildTurnDisclosureItems(
      [
        item('user', 'user', { createdAt: 1000 }),
        item('analysis-note', 'assistant', { createdAt: 1500 }),
        item('tool', 'process', { createdAt: 2000 }),
        item('summary', 'assistant', { publicText: true, sourceMessageIds: [SOURCE_1], turnOutcome: { kind: 'answered', message_ids: [SOURCE_1] }, createdAt: 4000 }),
      ],
      { tailClosed: true }
    );

    const disclosure = result.find((entry) => entry.type === 'turn_disclosure');
    expect(disclosure?.processItemIds).toEqual(['analysis-note', 'tool']);
    expect(result.map((entry) => entry.type === 'item' ? entry.id : entry.id)).toEqual([
      'user',
      DISCLOSURE_1,
      'summary',
    ]);
  });

  test('renders unfinished running process steps as a live turn disclosure before the final answer exists', () => {
    const result = buildTurnDisclosureItems([
      item('user', 'user', { createdAt: 1000 }),
      item('analysis', 'process', { createdAt: 2000, processStartedAt: 1500, processState: 'running' }),
      item('tool', 'process', { createdAt: 3000, processEndedAt: 3200 }),
    ]);

    expect(result.map((entry) => (entry.type === 'item' ? entry.id : entry.id))).toEqual([
      'user',
      DISCLOSURE_1,
    ]);
    const disclosure = result[1];
    expect(disclosure.type).toBe('turn_disclosure');
    if (disclosure.type !== 'turn_disclosure') return;
    expect(disclosure.state).toBe('running');
    expect(disclosure.running).toBe(true);
    expect(disclosure.defaultCollapsed).toBe(false);
    expect(disclosure.processItemIds).toEqual(['analysis', 'tool']);
    expect(disclosure.startAt).toBe(1000);
    expect(disclosure.endAt).toBe(3200);
  });

  test('keeps a live disclosure visible while the current turn waits for the first process item', () => {
    const result = buildTurnDisclosureItems([
      item('user', 'user', { createdAt: 1000 }),
    ]);

    expect(result.map((entry) => (entry.type === 'item' ? entry.id : entry.id))).toEqual([
      'user',
      DISCLOSURE_1,
    ]);
    const disclosure = result[1];
    expect(disclosure.type).toBe('turn_disclosure');
    if (disclosure.type !== 'turn_disclosure') return;
    expect(disclosure.state).toBe('running');
    expect(disclosure.running).toBe(true);
    expect(disclosure.defaultCollapsed).toBe(false);
    expect(disclosure.processItemIds).toEqual([]);
    expect(disclosure.sourceMessageIds).toEqual([]);
    expect(disclosure.startAt).toBe(1000);
    expect(disclosure.endAt).toBe(1000);
  });

  test('keeps the current turn disclosure visible between active process phases', () => {
    const result = buildTurnDisclosureItems([
      item('user', 'user', { createdAt: 1000 }),
      item('tool', 'process', { createdAt: 2000, processState: 'completed' }),
    ]);

    expect(result.map((entry) => (entry.type === 'item' ? entry.id : entry.id))).toEqual([
      'user',
      DISCLOSURE_1,
    ]);
    const disclosure = result[1];
    expect(disclosure.type).toBe('turn_disclosure');
    if (disclosure.type !== 'turn_disclosure') return;
    expect(disclosure.state).toBe('running');
    expect(disclosure.running).toBe(true);
    expect(disclosure.defaultCollapsed).toBe(false);
    expect(disclosure.processItemIds).toEqual(['tool']);
    expect(disclosure.processItemStates).toEqual({ tool: 'completed' });
  });

  test('keeps thinking items inside the process disclosure content', () => {
    const result = buildTurnDisclosureItems([
      item('user', 'user', { createdAt: 1000 }),
      item('thinking', 'process_content', { createdAt: 1500, processState: 'running' }),
      item('tool', 'process', { createdAt: 2000, processState: 'running' }),
    ]);

    expect(result.map((entry) => (entry.type === 'item' ? entry.id : entry.id))).toEqual([
      'user',
      DISCLOSURE_1,
    ]);
    const disclosure = result[1];
    expect(disclosure.type).toBe('turn_disclosure');
    if (disclosure.type !== 'turn_disclosure') return;
    expect(disclosure.processItemIds).toEqual(['thinking', 'tool']);
  });

  test('does not archive an empty disclosure when a turn closes without process items', () => {
    const result = buildTurnDisclosureItems(
      [
        item('user', 'user', { createdAt: 1000 }),
      ],
      { tailClosed: true }
    );

    expect(result).toEqual([{ type: 'item', id: 'user' }]);
  });

  test('settles stale running process steps after a closed turn has a final answer', () => {
    const result = buildTurnDisclosureItems(
      [
        item('user', 'user', { createdAt: 1000 }),
        item('tool', 'process', { createdAt: 2000, processState: 'running' }),
        item('final', 'assistant', { createdAt: 3000 }),
      ],
      { tailClosed: true }
    );

    expect(result.map((entry) => (entry.type === 'item' ? entry.id : entry.id))).toEqual([
      'user',
      DISCLOSURE_1,
      'final',
    ]);
    const disclosure = result[1];
    expect(disclosure.type).toBe('turn_disclosure');
    if (disclosure.type !== 'turn_disclosure') return;
    expect(disclosure.state).toBe('completed');
    expect(disclosure.processItemStates).toEqual({ tool: 'completed' });
  });

  test('settles stale running thinking when a process-only turn closes', () => {
    const result = buildTurnDisclosureItems(
      [
        item('user', 'user', { createdAt: 1000 }),
        item('thinking', 'process_content', { createdAt: 2000, processState: 'running' }),
      ],
      { tailClosed: true }
    );

    const disclosure = result[1];
    expect(disclosure.type).toBe('turn_disclosure');
    if (disclosure.type !== 'turn_disclosure') return;
    expect(disclosure.state).toBe('completed');
    expect(disclosure.running).toBe(false);
    expect(disclosure.processItemStates).toEqual({ thinking: 'completed' });
  });

  test('keeps provisional assistant text inside the live disclosure', () => {
    const result = buildTurnDisclosureItems([
      item('user', 'user', { createdAt: 1000 }),
      item('progress-note', 'assistant', { createdAt: 1500 }),
      item('scan', 'process', { createdAt: 2000, processState: 'running' }),
      item('partial-answer', 'assistant', { createdAt: 3000 }),
    ]);

    expect(result.map((entry) => (entry.type === 'item' ? entry.id : entry.id))).toEqual([
      'user',
      DISCLOSURE_1,
    ]);
    const disclosure = result[1];
    expect(disclosure.type).toBe('turn_disclosure');
    if (disclosure.type !== 'turn_disclosure') return;
    expect(disclosure.state).toBe('running');
    expect(disclosure.processItemIds).toEqual(['progress-note', 'scan', 'partial-answer']);
  });

  test('keeps active process steps visible in the live disclosure', () => {
    const result = buildTurnDisclosureItems([
      item('user', 'user', { createdAt: 1000 }),
      item('active-process', 'process', { createdAt: 2000, processState: 'running' }),
      item('partial-answer', 'assistant', { createdAt: 3000 }),
    ]);

    expect(result.map((entry) => (entry.type === 'item' ? entry.id : entry.id))).toEqual([
      'user',
      DISCLOSURE_1,
    ]);
    const disclosure = result[1];
    expect(disclosure.type).toBe('turn_disclosure');
    if (disclosure.type !== 'turn_disclosure') return;
    expect(disclosure.state).toBe('running');
    expect(disclosure.running).toBe(true);
    expect(disclosure.defaultCollapsed).toBe(false);
    expect(disclosure.processItemIds).toEqual(['active-process', 'partial-answer']);
  });

  test('keeps an intermediate failure in details but marks a closed answered turn as processed', () => {
    const result = buildTurnDisclosureItems(
      [
        item('user', 'user', { createdAt: 1000 }),
        item('tool', 'process', { createdAt: 2000, processState: 'failed' }),
        item('final', 'assistant', { createdAt: 3000 }),
      ],
      { tailClosed: true }
    );

    const disclosure = result[1];
    expect(disclosure.type).toBe('turn_disclosure');
    if (disclosure.type !== 'turn_disclosure') return;
    expect(disclosure.defaultCollapsed).toBe(true);
    expect(disclosure.state).toBe('completed');
    expect(disclosure.processItemStates).toEqual({ tool: 'failed' });
  });

  test('keeps a closed failed process-only turn failed while retaining details', () => {
    const result = buildTurnDisclosureItems(
      [
        item('user', 'user', { createdAt: 1000 }),
        item('tool', 'process', {
          createdAt: 3000,
          processStartedAt: 1500,
          processEndedAt: 3000,
          processState: 'failed',
        }),
      ],
      { tailClosed: true }
    );

    const disclosure = result[1];
    expect(disclosure.type).toBe('turn_disclosure');
    if (disclosure.type !== 'turn_disclosure') return;
    expect(disclosure.state).toBe('failed');
    expect(disclosure.running).toBe(false);
    expect(disclosure.startAt).toBe(1000);
    expect(disclosure.endAt).toBe(3000);
    expect(disclosure.processItemStates).toEqual({ tool: 'failed' });
  });

  test('keeps an in-flight turn processing after an intermediate failure', () => {
    const result = buildTurnDisclosureItems([
      item('user', 'user', { createdAt: 1000 }),
      item('tool', 'process', { createdAt: 2000, processState: 'failed' }),
    ]);

    const disclosure = result[1];
    expect(disclosure.type).toBe('turn_disclosure');
    if (disclosure.type !== 'turn_disclosure') return;
    expect(disclosure.state).toBe('running');
    expect(disclosure.running).toBe(true);
    expect(disclosure.processItemStates).toEqual({ tool: 'failed' });
  });

  test('keeps a canceled closed turn and its execution interval', () => {
    const result = buildTurnDisclosureItems(
      [
        item('user', 'user', { createdAt: 1000 }),
        item('tool', 'process', {
          createdAt: 5200,
          processStartedAt: 1200,
          processEndedAt: 5200,
          processState: 'canceled',
        }),
      ],
      { tailClosed: true }
    );

    const disclosure = result[1];
    expect(disclosure.type).toBe('turn_disclosure');
    if (disclosure.type !== 'turn_disclosure') return;
    expect(disclosure.state).toBe('canceled');
    expect(disclosure.running).toBe(false);
    expect(disclosure.startAt).toBe(1000);
    expect(disclosure.endAt).toBe(5200);
    expect(disclosure.processItemStates).toEqual({ tool: 'canceled' });
  });

  test('lets a final cancellation override an earlier failed process item', () => {
    const result = buildTurnDisclosureItems(
      [
        item('user', 'user', { createdAt: 1000 }),
        item('failed-tool', 'process', {
          createdAt: 2800,
          processStartedAt: 1200,
          processEndedAt: 2800,
          processState: 'failed',
        }),
        item('canceled-tool', 'process', {
          createdAt: 6200,
          processStartedAt: 3000,
          processEndedAt: 6200,
          processState: 'canceled',
        }),
      ],
      { tailClosed: true }
    );

    const disclosure = result[1];
    expect(disclosure.type).toBe('turn_disclosure');
    if (disclosure.type !== 'turn_disclosure') return;
    expect(disclosure.state).toBe('canceled');
    expect(disclosure.startAt).toBe(1000);
    expect(disclosure.endAt).toBe(6200);
    expect(disclosure.processItemStates).toEqual({
      'failed-tool': 'failed',
      'canceled-tool': 'canceled',
    });
  });

  test('keeps a completed process-only tail inside the live disclosure until the request closes', () => {
    const result = buildTurnDisclosureItems([
      item('user', 'user', { createdAt: 1000 }),
      item('tool', 'process', { createdAt: 2000, processState: 'completed' }),
    ]);

    expect(result.map((entry) => (entry.type === 'item' ? entry.id : entry.id))).toEqual([
      'user',
      DISCLOSURE_1,
    ]);
    const disclosure = result[1];
    expect(disclosure.type).toBe('turn_disclosure');
    if (disclosure.type !== 'turn_disclosure') return;
    expect(disclosure.state).toBe('running');
    expect(disclosure.processItemIds).toEqual(['tool']);
  });

  test('keeps a completed tail and provisional assistant text in the live disclosure', () => {
    const result = buildTurnDisclosureItems([
      item('user', 'user', { createdAt: 1000 }),
      item('tool', 'process', { createdAt: 2000, processState: 'completed' }),
      item('assistant-text', 'assistant', { createdAt: 3000 }),
    ]);

    expect(result.map((entry) => (entry.type === 'item' ? entry.id : entry.id))).toEqual([
      'user',
      DISCLOSURE_1,
    ]);
    const disclosure = result[1];
    expect(disclosure.type).toBe('turn_disclosure');
    if (disclosure.type !== 'turn_disclosure') return;
    expect(disclosure.state).toBe('running');
    expect(disclosure.processItemIds).toEqual(['tool', 'assistant-text']);
  });

  test('settles a completed process-only segment once the next request closes it', () => {
    const result = buildTurnDisclosureItems(
      [
        item('user-1', 'user', { turnId: TURN_1, createdAt: 1000 }),
        item('tool-1', 'process', { turnId: TURN_1, createdAt: 2000, processState: 'completed' }),
        item('user-2', 'user', { turnId: TURN_2, createdAt: 3000 }),
      ],
      { tailClosed: true }
    );

    expect(result.map((entry) => (entry.type === 'item' ? entry.id : entry.id))).toEqual([
      'user-1',
      DISCLOSURE_1,
      'user-2',
    ]);
    const disclosure = result[1];
    expect(disclosure.type).toBe('turn_disclosure');
    if (disclosure.type !== 'turn_disclosure') return;
    expect(disclosure.defaultCollapsed).toBe(true);
    expect(disclosure.state).toBe('completed');
    expect(disclosure.processItemIds).toEqual(['tool-1']);
  });

  test('keeps completed disclosures scoped to their own turn id', () => {
    const result = buildTurnDisclosureItems(
      [
        item('user-1', 'user', { turnId: TURN_1, createdAt: 1000 }),
        item('tool-1', 'process', { turnId: TURN_1, createdAt: 2000 }),
        item('final-1', 'assistant', { turnId: TURN_1, createdAt: 3000 }),
        item('user-2', 'user', { turnId: TURN_2, createdAt: 4000 }),
        item('tool-2', 'process', { turnId: TURN_2, createdAt: 5000 }),
        item('final-2', 'assistant', { turnId: TURN_2, createdAt: 6000 }),
      ],
      { tailClosed: true }
    );

    expect(result.map((entry) => (entry.type === 'item' ? entry.id : entry.id))).toEqual([
      'user-1',
      DISCLOSURE_1,
      'final-1',
      'user-2',
      DISCLOSURE_2,
      'final-2',
    ]);
    expect(result.filter((entry) => entry.type === 'turn_disclosure')).toHaveLength(2);
  });

  test('folds a delayed process fragment into the first disclosure without reordering visible messages', () => {
    const result = buildTurnDisclosureItems(
      [
        item('user-1', 'user', { turnId: TURN_1, createdAt: 1000 }),
        item('tool-1', 'process', { turnId: TURN_1, createdAt: 2000 }),
        item('final-1', 'assistant', { turnId: TURN_1, createdAt: 3000 }),
        item('user-2', 'user', { turnId: TURN_2, createdAt: 4000 }),
        item('tool-2', 'process', { turnId: TURN_2, createdAt: 5000 }),
        item('final-2', 'assistant', { turnId: TURN_2, createdAt: 6000 }),
        item('late-tool-1', 'process', { turnId: TURN_1, createdAt: 7000 }),
      ],
      { tailClosed: true }
    );

    expect(result.map((entry) => entry.id)).toEqual([
      'user-1',
      DISCLOSURE_1,
      'final-1',
      'user-2',
      DISCLOSURE_2,
      'final-2',
    ]);
    expect(new Set(result.map((entry) => entry.id)).size).toBe(result.length);
    const firstDisclosure = result.find(
      (entry) => entry.type === 'turn_disclosure' && entry.turnId === TURN_1
    );
    expect(firstDisclosure?.type).toBe('turn_disclosure');
    if (firstDisclosure?.type !== 'turn_disclosure') return;
    expect(firstDisclosure.processItemIds).toEqual(['tool-1', 'late-tool-1']);
    expect(firstDisclosure.endAt).toBe(3000);
  });

  test('places delayed persisted process rows before each turn final answer instead of stacking them at the tail', () => {
    const result = buildTurnDisclosureItems(
      [
        item('user-1', 'user', { turnId: TURN_1, createdAt: 1000 }),
        item('final-1', 'assistant', { turnId: TURN_1, createdAt: 2000 }),
        item('user-2', 'user', { turnId: TURN_2, createdAt: 3000 }),
        item('final-2', 'assistant', { turnId: TURN_2, createdAt: 4000 }),
        item('persisted-process-1', 'process', { turnId: TURN_1, createdAt: 1500 }),
        item('persisted-process-2', 'process', { turnId: TURN_2, createdAt: 3500 }),
      ],
      { tailClosed: true }
    );

    expect(result.map((entry) => entry.id)).toEqual([
      'user-1',
      DISCLOSURE_1,
      'final-1',
      'user-2',
      DISCLOSURE_2,
      'final-2',
    ]);
    const disclosures = result.filter((entry) => entry.type === 'turn_disclosure');
    expect(disclosures).toHaveLength(2);
    expect(new Set(disclosures.map((entry) => entry.turnId))).toEqual(new Set([TURN_1, TURN_2]));
  });

  test('selects final assistant content across non-contiguous fragments of the same turn', () => {
    const result = buildTurnDisclosureItems(
      [
        item('user-1', 'user', { turnId: TURN_1, createdAt: 1000 }),
        item('intro-1', 'assistant', { turnId: TURN_1, createdAt: 1500 }),
        item('tool-1', 'process', { turnId: TURN_1, createdAt: 2000 }),
        item('delayed-turn-2', 'process', { turnId: TURN_2, createdAt: 2500 }),
        item('final-1', 'assistant', { publicText: true, sourceMessageIds: [SOURCE_1], turnOutcome: { kind: 'answered', message_ids: [SOURCE_1] }, turnId: TURN_1, createdAt: 3000 }),
      ],
      { tailClosed: true }
    );

    expect(result.map((entry) => entry.id)).toEqual([
      'user-1',
      DISCLOSURE_1,
      DISCLOSURE_2,
      'final-1',
    ]);
    const disclosure = result.find(
      (entry) => entry.type === 'turn_disclosure' && entry.turnId === TURN_1
    );
    expect(disclosure?.type).toBe('turn_disclosure');
    if (disclosure?.type !== 'turn_disclosure') return;
    expect(disclosure.processItemIds).toEqual(['intro-1', 'tool-1']);
    expect(disclosure.endAt).toBe(3000);
    expect(disclosure.defaultCollapsed).toBe(true);
  });

  test('uses the latest fragment state when a canceled turn later completes', () => {
    const result = buildTurnDisclosureItems(
      [
        item('user-1', 'user', { turnId: TURN_1, createdAt: 1000 }),
        item('canceled-1', 'process', {
          turnId: TURN_1,
          createdAt: 2000,
          processState: 'canceled',
        }),
        item('delayed-turn-2', 'process', { turnId: TURN_2, createdAt: 2500 }),
        item('completed-1', 'process', {
          turnId: TURN_1,
          createdAt: 3000,
          processState: 'completed',
        }),
        item('final-1', 'assistant', { turnId: TURN_1, createdAt: 3500 }),
      ],
      { tailClosed: true }
    );

    const disclosure = result.find(
      (entry) => entry.type === 'turn_disclosure' && entry.turnId === TURN_1
    );
    expect(disclosure?.type).toBe('turn_disclosure');
    if (disclosure?.type !== 'turn_disclosure') return;
    expect(disclosure.state).toBe('completed');
    expect(disclosure.processItemStates).toEqual({
      'canceled-1': 'canceled',
      'completed-1': 'completed',
    });
  });

  test('uses the latest fragment state when an active turn resumes running', () => {
    const result = buildTurnDisclosureItems(
      [
        item('user-1', 'user', { turnId: TURN_1, createdAt: 1000 }),
        item('active-1', 'process', {
          turnId: TURN_1,
          createdAt: 2000,
          processState: 'running',
        }),
        item('delayed-turn-2', 'process', { turnId: TURN_2, createdAt: 2500 }),
        item('running-1', 'process', {
          turnId: TURN_1,
          createdAt: 3000,
          processState: 'running',
        }),
      ],
      { activeTurnId: TURN_1 }
    );

    const disclosure = result.find(
      (entry) => entry.type === 'turn_disclosure' && entry.turnId === TURN_1
    );
    expect(disclosure?.type).toBe('turn_disclosure');
    if (disclosure?.type !== 'turn_disclosure') return;
    expect(disclosure.state).toBe('running');
    expect(disclosure.running).toBe(true);
    expect(disclosure.defaultCollapsed).toBe(false);
  });

  test('does not let an older process state win merely because every fragment includes the global final time', () => {
    const result = buildTurnDisclosureItems(
      [
        item('user-1', 'user', { turnId: TURN_1, createdAt: 1000 }),
        item('completed-1', 'process', {
          turnId: TURN_1,
          createdAt: 5000,
          processState: 'completed',
        }),
        item('final-1', 'assistant', { turnId: TURN_1, createdAt: 6000 }),
        item('delayed-turn-2', 'process', { turnId: TURN_2, createdAt: 6500 }),
        item('stale-canceled-1', 'process', {
          turnId: TURN_1,
          createdAt: 2000,
          processState: 'canceled',
        }),
      ],
      { tailClosed: true }
    );

    const disclosure = result.find(
      (entry) => entry.type === 'turn_disclosure' && entry.turnId === TURN_1
    );
    expect(disclosure?.type).toBe('turn_disclosure');
    if (disclosure?.type !== 'turn_disclosure') return;
    expect(disclosure.endAt).toBe(6000);
    expect(disclosure.state).toBe('completed');
    expect(disclosure.processItemStates['stale-canceled-1']).toBe('canceled');
  });

  test('does not mistake a delayed older assistant row for the final answer merely because it was appended last', () => {
    const result = buildTurnDisclosureItems(
      [
        item('user-1', 'user', { turnId: TURN_1, createdAt: 1000 }),
        item('tool-1', 'process', { turnId: TURN_1, createdAt: 2000 }),
        item('true-final-1', 'assistant', { publicText: true, sourceMessageIds: [SOURCE_1], turnOutcome: { kind: 'answered', message_ids: [SOURCE_1] }, turnId: TURN_1, createdAt: 6000 }),
        item('delayed-turn-2', 'process', { turnId: TURN_2, createdAt: 6500 }),
        item('late-old-intro-1', 'assistant', { turnId: TURN_1, createdAt: 3000 }),
      ],
      { tailClosed: true }
    );

    expect(result.map((entry) => entry.id)).toEqual([
      'user-1',
      DISCLOSURE_1,
      'true-final-1',
      DISCLOSURE_2,
    ]);
    const disclosure = result.find(
      (entry) => entry.type === 'turn_disclosure' && entry.turnId === TURN_1
    );
    expect(disclosure?.type).toBe('turn_disclosure');
    if (disclosure?.type !== 'turn_disclosure') return;
    expect(disclosure.processItemIds).toEqual(['tool-1', 'late-old-intro-1']);
    expect(disclosure.endAt).toBe(6000);
  });

  test('keeps the latest user turn running when an older turn fragment arrives last', () => {
    const result = buildTurnDisclosureItems(
      [
        item('user-1', 'user', { turnId: TURN_1, createdAt: 1000 }),
        item('tool-1', 'process', { turnId: TURN_1, createdAt: 2000 }),
        item('final-1', 'assistant', { turnId: TURN_1, createdAt: 3000 }),
        item('user-2', 'user', { turnId: TURN_2, createdAt: 4000 }),
        item('tool-2', 'process', { turnId: TURN_2, createdAt: 5000, processState: 'running' }),
        item('late-tool-1', 'process', { turnId: TURN_1, createdAt: 6000 }),
      ],
      { activeTurnId: TURN_2 }
    );

    const disclosures = result.filter((entry) => entry.type === 'turn_disclosure');
    expect(disclosures).toHaveLength(2);
    expect(disclosures.find((entry) => entry.turnId === TURN_1)?.state).toBe('completed');
    expect(disclosures.find((entry) => entry.turnId === TURN_2)?.state).toBe('running');
  });

  test('uses an explicit active turn for background work without a visible user row', () => {
    const result = buildTurnDisclosureItems(
      [
        item('current-background-tool', 'process', {
          turnId: TURN_2,
          createdAt: 1000,
          processState: 'running',
        }),
        item('delayed-old-tool', 'process', { turnId: TURN_1, createdAt: 2000 }),
      ],
      { activeTurnId: TURN_2 }
    );

    const disclosures = result.filter((entry) => entry.type === 'turn_disclosure');
    expect(disclosures.find((entry) => entry.turnId === TURN_2)?.state).toBe('running');
    expect(disclosures.find((entry) => entry.turnId === TURN_1)?.state).toBe('completed');
  });

  test('renders process steps without a visible user request as inline receipts', () => {
    const result = buildTurnDisclosureItems([
      item('scan', 'process', { turnId: undefined, createdAt: 1000, processState: 'completed' }),
      item('tool', 'process', { turnId: undefined, createdAt: 1500, processState: 'completed' }),
      item('assistant-text', 'assistant', { turnId: undefined, createdAt: 2000 }),
    ]);

    expect(result).toEqual([
      { type: 'process_receipt', id: 'receipt-scan', itemId: 'scan' },
      { type: 'process_receipt', id: 'receipt-tool', itemId: 'tool' },
      { type: 'item', id: 'assistant-text' },
    ]);
  });
});

describe('assignTurnIdsFromUserRequests', () => {
  test('promotes the backend root turn id over the distinct user request id for unowned tail rows', () => {
    const assigned = assignTurnIdsFromUserRequests([
      item('user', 'user', { turnId: TURN_1, createdAt: 1000 }),
      item('tool', 'process', { turnId: AGENT_ROOT_1, createdAt: 2000, processState: 'completed' }),
      item('final', 'assistant', { turnId: AGENT_ROOT_1, createdAt: 3000 }),
      item('terminal-orphan', 'process', {
        turnId: undefined,
        createdAt: 3001,
        processState: 'completed',
      }),
    ]);

    expect(assigned.map((entry) => entry.turnId)).toEqual([
      AGENT_ROOT_1,
      AGENT_ROOT_1,
      AGENT_ROOT_1,
      AGENT_ROOT_1,
    ]);

    const display = buildTurnDisclosureItems(assigned, { tailClosed: true });
    const disclosures = display.filter((entry) => entry.type === 'turn_disclosure');
    expect(disclosures).toHaveLength(1);
    expect(disclosures[0]?.turnId).toBe(AGENT_ROOT_1);
    expect(disclosures[0]?.processItemIds).toEqual(['tool', 'terminal-orphan']);
    expect(display.map((entry) => entry.id)).toEqual([
      'user',
      `turn-disclosure-${AGENT_ROOT_1}`,
      'final',
    ]);
    expect(new Set(display.map((entry) => entry.id)).size).toBe(display.length);
  });

  test('groups all assistant and process messages after one user request into the same turn', () => {
    const result = assignTurnIdsFromUserRequests([
      item('user', 'user', { turnId: TURN_1, createdAt: 1000 }),
      item('scan', 'process', { turnId: undefined, createdAt: 1500 }),
      item('progress', 'assistant', { turnId: undefined, createdAt: 2000 }),
      item('tool', 'process', { turnId: undefined, createdAt: 2500 }),
      item('final', 'assistant', { turnId: undefined, createdAt: 3000 }),
    ]);

    expect(result.map((entry) => entry.turnId)).toEqual([TURN_1, TURN_1, TURN_1, TURN_1, TURN_1]);
  });

  test('starts a new request group at the next user message and leaves leading system items ungrouped', () => {
    const result = assignTurnIdsFromUserRequests([
      item('status', 'other', { turnId: undefined, createdAt: 500 }),
      item('user-1', 'user', { turnId: TURN_1, createdAt: 1000 }),
      item('tool-1', 'process', { turnId: undefined, createdAt: 1500 }),
      item('user-2', 'user', { turnId: TURN_2, createdAt: 2000 }),
      item('tool-2', 'process', { turnId: undefined, createdAt: 2500 }),
    ]);

    expect(result.map((entry) => entry.turnId)).toEqual([undefined, TURN_1, TURN_1, TURN_2, TURN_2]);
  });

  test('preserves an authoritative delayed turn id without poisoning the positional fallback', () => {
    const result = assignTurnIdsFromUserRequests([
      item('user-1', 'user', { turnId: TURN_1, createdAt: 1000 }),
      item('user-2', 'user', { turnId: TURN_2, createdAt: 2000 }),
      item('delayed-error-1', 'assistant', { turnId: TURN_1, createdAt: 3000 }),
      item('legacy-tool-2', 'process', { turnId: undefined, createdAt: 4000 }),
    ]);

    expect(result.map((entry) => entry.turnId)).toEqual([TURN_1, TURN_2, TURN_1, TURN_2]);
  });

  test('does not promote a retired provisional request id before the current root arrives', () => {
    const userTurn2 = parseMessageId('0190f5fe-7c00-7a00-8000-000000000022');
    const result = assignTurnIdsFromUserRequests([
      item('user-1', 'user', { turnId: TURN_1, createdAt: 1000 }),
      item('root-1', 'process', { turnId: AGENT_ROOT_1, createdAt: 1500 }),
      item('user-2', 'user', { turnId: userTurn2, createdAt: 2000 }),
      item('delayed-provisional-1', 'assistant', { turnId: TURN_1, createdAt: 2500 }),
      item('legacy-tool-2', 'process', { turnId: undefined, createdAt: 3000 }),
    ]);

    expect(result.map((entry) => entry.turnId)).toEqual([
      AGENT_ROOT_1,
      AGENT_ROOT_1,
      userTurn2,
      TURN_1,
      userTurn2,
    ]);
  });

  test('uses the active request correlation before the first root-owned stream row arrives', () => {
    const assigned = assignTurnIdsFromUserRequests(
      [
        item('user', 'user', { turnId: TURN_1, createdAt: 1000 }),
        item('early-thinking', 'process_content', {
          turnId: undefined,
          createdAt: 1200,
          processState: 'running',
        }),
      ],
      { activeRequestMessageId: TURN_1, activeTurnId: AGENT_ROOT_1 }
    );

    expect(assigned.map((entry) => entry.turnId)).toEqual([AGENT_ROOT_1, AGENT_ROOT_1]);
    const display = buildTurnDisclosureItems(assigned, { activeTurnId: AGENT_ROOT_1 });
    expect(display.map((entry) => entry.id)).toEqual([
      'user',
      `turn-disclosure-${AGENT_ROOT_1}`,
    ]);
    expect(display[1]?.type).toBe('turn_disclosure');
    if (display[1]?.type !== 'turn_disclosure') return;
    expect(display[1].state).toBe('running');
    expect(display[1].processItemIds).toEqual(['early-thinking']);
  });

  test('does not let an unseen delayed old turn override an authoritative active request pair', () => {
    const assigned = assignTurnIdsFromUserRequests(
      [
        item('user', 'user', { turnId: TURN_1, createdAt: 1000 }),
        item('delayed-old-event', 'process', { turnId: TURN_2, createdAt: 1100 }),
        item('current-unowned-thinking', 'process_content', {
          turnId: undefined,
          createdAt: 1200,
          processState: 'running',
        }),
      ],
      { activeRequestMessageId: TURN_1, activeTurnId: AGENT_ROOT_1 }
    );

    expect(assigned.map((entry) => entry.turnId)).toEqual([AGENT_ROOT_1, TURN_2, AGENT_ROOT_1]);
  });
});

describe('stop notice', () => {
  test('marks the closed tail turn canceled and pins endAt to the stop moment', () => {
    const output = buildTurnDisclosureItems(
      [item('u1', 'user', { createdAt: 1_000 }), item('p1', 'process', { createdAt: 2_000 })],
      { tailClosed: true, stopNotice: { stoppedAt: 5_000 } }
    );
    const disclosure = output.find((entry) => entry.type === 'turn_disclosure');
    expect(disclosure?.type).toBe('turn_disclosure');
    if (disclosure?.type !== 'turn_disclosure') return;
    expect(disclosure.state).toBe('canceled');
    expect(disclosure.endAt).toBe(5_000);
    expect(disclosure.running).toBe(false);
    expect(disclosure.defaultCollapsed).toBe(true);
  });

  test('ignores a stop notice that predates the tail turn', () => {
    const output = buildTurnDisclosureItems(
      [item('u1', 'user', { createdAt: 1_000 }), item('p1', 'process', { createdAt: 2_000 })],
      { tailClosed: true, stopNotice: { stoppedAt: 500 } }
    );
    const disclosure = output.find((entry) => entry.type === 'turn_disclosure');
    expect(disclosure?.type).toBe('turn_disclosure');
    if (disclosure?.type !== 'turn_disclosure') return;
    expect(disclosure.state).toBe('completed');
    expect(disclosure.endAt).toBe(2_000);
  });

  test('does not cancel a still-running tail turn', () => {
    const output = buildTurnDisclosureItems(
      [item('u1', 'user', { createdAt: 1_000 }), item('p1', 'process', { createdAt: 2_000, running: true })],
      { tailClosed: false, stopNotice: { stoppedAt: 5_000 } }
    );
    const disclosure = output.find((entry) => entry.type === 'turn_disclosure');
    expect(disclosure?.type).toBe('turn_disclosure');
    if (disclosure?.type !== 'turn_disclosure') return;
    expect(disclosure.state).toBe('running');
    expect(disclosure.running).toBe(true);
  });
});

test('a late paused status in a different row cannot replace the public terminal handoff', () => {
  const question = item('question', 'assistant', { userInputRequestId: 'ask:profile',
    turnOutcome: { kind: 'input_required', request_id: 'ask:profile' } });
  const paused = item('late-paused-summary', 'metadata', { turnOutcome: { kind: 'paused', message_ids: [] }, createdAt: 999999 });
  expect([...selectTurnPublicResultIds([question, paused])]).toEqual(['question']);
  expect([...selectTurnPublicResultIds([paused, question])]).toEqual(['question']);
});

test('source-bound process records can open a paged-out tool Turn without inventing an end time', () => {
  const turnId = parseMessageId('0190f5fe-7c00-7a00-8000-000000008881');
  const items: TurnDisclosureInputItem[] = [{ id: 'request-only', role: 'user', turnId, createdAt: 10, displayAt: 1_000 }];
  const output = buildTurnDisclosureItems(items, { tailClosed: true, processRecordTurnIds: new Set([turnId]) });
  const process = output.find(entry => entry.type === 'turn_disclosure');
  expect(process?.type).toBe('turn_disclosure');
  if (process?.type !== 'turn_disclosure') throw new Error('Expected exact source process');
  expect(process.processItemIds).toEqual([]);
  expect(process.startAt).toBe(1_000);
  expect(Number.isNaN(process.endAt)).toBe(true);
  expect(buildTurnDisclosureItems(items, { tailClosed: true }).some(entry => entry.type === 'turn_disclosure')).toBe(false);
});

test('an exact steering input remains in its original Turn without changing the root for transient tail rows', () => {
  const assigned = assignTurnIdsFromUserRequests([
    item('root', 'user', { inputKind: 'root', turnId: TURN_1 }),
    item('before', 'process', { turnId: TURN_1 }),
    item('guide', 'user', { inputKind: 'steering', turnId: TURN_1 }),
    item('late-other-turn', 'process', { turnId: TURN_2 }),
    item('after', 'process', { turnId: undefined }),
  ]);
  expect(assigned.map(entry => entry.turnId)).toEqual([TURN_1, TURN_1, TURN_1, TURN_2, TURN_1]);
});

test('steering keeps earlier and later process on opposite sides and delayed facts join only their own boundary', () => {
  const guide = item('guide', 'user', { inputKind: 'steering', sequence: 40 });
  const entries = [
    item('root', 'user', { inputKind: 'root', sequence: 10 }),
    item('before', 'process', { sequence: 20 }),
    guide,
    item('after', 'process', { sequence: 50 }),
    item('answer', 'assistant', { sequence: 60 }),
    item('late-before', 'process', { sequence: 30 }),
    item('late-after', 'process', { sequence: 55 }),
  ];
  const output = buildTurnDisclosureItems(entries, { tailClosed: true });
  expect(output.map(entry => entry.id)).toEqual(['root', DISCLOSURE_1, 'guide', `${DISCLOSURE_1}:guide`, 'answer']);
  const disclosures = output.filter(entry => entry.type === 'turn_disclosure');
  expect(disclosures.map(entry => entry.turnId)).toEqual([TURN_1, TURN_1]);
  expect(disclosures.map(entry => entry.processItemIds)).toEqual([['before', 'late-before'], ['after', 'late-after']]);
  expect(new Set(output.map(entry => entry.id)).size).toBe(output.length);
});

test('a current intervention closes only its preceding process while the same Turn continues', () => {
  const output = buildTurnDisclosureItems([
    item('root', 'user', { inputKind: 'root', sequence: 10 }),
    item('before', 'process_content', { sequence: 20, processState: 'running' }),
    item('guide', 'user', { inputKind: 'steering', sequence: 30 }),
    item('after', 'process_content', { sequence: 40, processState: 'running' }),
  ], { activeTurnId: TURN_1, tailClosed: false });
  expect(output.map(entry => entry.id)).toEqual(['root', DISCLOSURE_1, 'guide', `${DISCLOSURE_1}:guide`]);
  const disclosures = output.filter(entry => entry.type === 'turn_disclosure');
  expect(disclosures.map(entry => entry.running)).toEqual([false, true]);
  expect(disclosures.map(entry => entry.earlierSegment === true)).toEqual([true, false]);
  expect(disclosures[0].processItemStates.before).toBe('completed');
  expect(disclosures[1].processItemStates.after).toBe('running');
});

test('opening steering without prior process shares one initial process instead of introducing an empty segment', () => {
  const output = buildTurnDisclosureItems([
    item('root', 'user', { inputKind: 'root', sequence: 10 }),
    item('opening-guide', 'user', { inputKind: 'steering', sequence: 20 }),
    item('after', 'process', { sequence: 30, processState: 'running' }),
  ], { activeTurnId: TURN_1 });
  expect(output.filter(entry => entry.type === 'turn_disclosure')).toHaveLength(1);
  expect(output.map(entry => entry.id)).toEqual(['root', 'opening-guide', DISCLOSURE_1]);
});

test('multiple interventions retain distinct reading boundaries and a later cancellation settles only the active segment', () => {
  const output = buildTurnDisclosureItems([
    item('root', 'user', { inputKind: 'root', sequence: 10 }),
    item('first', 'process', { sequence: 20, processState: 'completed' }),
    item('guide-one', 'user', { inputKind: 'steering', sequence: 30 }),
    item('middle', 'process', { sequence: 40, processState: 'completed' }),
    item('guide-two', 'user', { inputKind: 'steering', sequence: 50 }),
    item('last', 'process', { sequence: 60, processState: 'running' }),
    item('terminal', 'metadata', { sequence: 70, turnStartedAt: 1000, turnEndedAt: 7000, processState: 'canceled' }),
  ], { tailClosed: true });
  expect(output.map(entry => entry.id)).toEqual(['root', DISCLOSURE_1, 'guide-one', `${DISCLOSURE_1}:guide-one`,
    'guide-two', `${DISCLOSURE_1}:guide-two`]);
  const disclosures = output.filter(entry => entry.type === 'turn_disclosure');
  expect(disclosures.map(entry => entry.state)).toEqual(['completed', 'completed', 'canceled']);
  expect(disclosures.map(entry => entry.processItemIds)).toEqual([['first'], ['middle'], ['last']]);
});
