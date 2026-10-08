/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type { IResponseMessage } from '@/common/adapter/ipcBridge';
import type { TMessage } from '@/common/chat/chatLib';
import { parseConversationId, parseMessageId } from '@/common/types/ids';
import {
  beginCompanionSessionBubble, clearCompanionSessionBubble, emptyCompanionSessionBubble,
  hydrateCompanionSessionBubble, settleCompanionSessionBubble, streamCompanionSessionBubble,
} from './companionSessionBubble';

const conversationId = parseConversationId('019f0000-0000-7000-8000-000000000901');
const otherConversation = parseConversationId('019f0000-0000-7000-8000-000000000902');
const root = parseMessageId('019f0000-0000-7000-8000-000000000903');
const nextRoot = parseMessageId('019f0000-0000-7000-8000-000000000904');
const segment = parseMessageId('019f0000-0000-7000-8000-000000000905');
const secondSegment = parseMessageId('019f0000-0000-7000-8000-000000000906');
const frame = (content: string, extra: Partial<IResponseMessage> = {}): IResponseMessage => ({
  conversation_id: conversationId, turn_id: root, msg_id: segment, type: 'content', data: { content }, ...extra,
});
const stream = (state: ReturnType<typeof emptyCompanionSessionBubble>, message: IResponseMessage) =>
  streamCompanionSessionBubble(state, message, 'Using tools', 'Provider error');
const running = () => beginCompanionSessionBubble(emptyCompanionSessionBubble(conversationId), root, 'running');

describe('canonical companion Turn bubble presentation', () => {
  test('uses exact Session and Turn identities without sending-window or source markers', () => {
    const state = stream(running(), frame('Hello'));
    expect(state.bubble).toBe('Hello');
    expect(stream(state, frame('Other', { conversation_id: otherConversation }))).toBe(state);
    expect(stream(state, frame('Older', { turn_id: nextRoot }))).toBe(state);
    expect(stream(state, frame('Projection', { stream_complete: true }))).toBe(state);
    expect(state.phase).toBe('running');
  });
  test('keeps activity through thinking, tools, output rollback, and stream terminal hints', () => {
    let state = stream(running(), frame('', { type: 'thinking' }));
    expect(state.bubble).toBe('…');
    state = stream(state, frame('', { type: 'tool_call' }));
    expect(state.bubble).toBe('Using tools');
    state = stream(state, frame('prefix'));
    state = stream(state, frame('', { type: 'output_discarded' }));
    expect(state.bubble).toBe('prefix');
    state = stream(state, frame('', { type: 'finish' }));
    expect(state.phase).toBe('running');
    expect(state.bubble).toBe('prefix');
  });
  test('replaces and hides only the exact segment while preserving other model steps', () => {
    let state = stream(running(), frame('One'));
    state = stream(state, frame(' two'));
    state = stream(state, frame('Three', { msg_id: secondSegment }));
    expect(state.bubble).toBe('One two\n\nThree');
    state = stream(state, frame('Correct', { replace: true }));
    expect(state.bubble).toBe('Correct\n\nThree');
    state = stream(state, frame('', { hidden: true }));
    expect(state.bubble).toBe('Three');
    state = stream(state, frame('', { msg_id: secondSegment, replace: true }));
    expect(state.bubble).toBe('…');
  });
  test('settlement and delayed clearing cannot touch a successor Turn', () => {
    let state = stream(running(), frame('First'));
    expect(settleCompanionSessionBubble(state, nextRoot, 'Done')).toBe(state);
    state = settleCompanionSessionBubble(state, root, 'Done');
    const finished = state;
    expect(beginCompanionSessionBubble(state, root, 'running')).toBe(finished);
    state = stream(state, frame('Final', { replace: true }));
    expect(state.phase).toBe('settled');
    expect(state.bubble).toBe('Final');
    state = beginCompanionSessionBubble(state, nextRoot, 'running');
    expect(clearCompanionSessionBubble(state, root)).toBe(state);
    expect(settleCompanionSessionBubble(state, root, 'Old')).toBe(state);
    expect(state.bubble).toBe('…');
  });
  test('dismissal and interruption suppression end at the next canonical Turn', () => {
    const state = { ...stream(running(), frame('First')), dismissed: true, bubble: '' };
    expect(stream(state, frame('Late'))).toBe(state);
    const successor = beginCompanionSessionBubble(state, nextRoot, 'running');
    expect(successor.dismissed).toBe(false);
    expect(stream(successor, frame('Second', { turn_id: nextRoot })).bubble).toBe('Second');
    const interrupted = { ...running(), interrupted: true, bubble: 'Keep this' };
    expect(stream(interrupted, frame('Late'))).toBe(interrupted);
  });
  test('accepted broadcasts never lower running authority and paused roots can resume', () => {
    const state = running();
    expect(beginCompanionSessionBubble(state, root, 'accepted')).toBe(state);
    const paused = settleCompanionSessionBubble(state, root, 'Paused', true);
    expect(beginCompanionSessionBubble(paused, root, 'accepted')).toBe(paused);
    expect(beginCompanionSessionBubble(paused, root, 'running').phase).toBe('running');
  });
  test('hydrates only the exact Turn and preserves segments changed during the read', () => {
    let state = stream(running(), frame('Live'));
    const observed = state;
    state = stream(state, frame(' answer'));
    const saved = (id = segment, turn = root, content = 'Saved'): TMessage => ({
      id: String(id), message_id: id, msg_id: id, conversation_id: conversationId, turn_id: turn,
      position: 'left', type: 'text', content: { content }, created_at: 100,
    });
    state = hydrateCompanionSessionBubble(state, [saved(), saved(secondSegment, nextRoot, 'Wrong Turn')], observed);
    expect(state.bubble).toBe('Live answer');
    state = hydrateCompanionSessionBubble(state, [saved(secondSegment, root, 'Another step')], state);
    expect(state.bubble).toContain('Another step');
    expect(state.bubble).toContain('Live answer');
    expect(state.bubble).not.toContain('Wrong Turn');
    expect(hydrateCompanionSessionBubble(state, [saved(segment, root, 'L')], state).bubble).toBe(state.bubble);
  });
  test('a late history snapshot cannot resurrect a hidden segment or undo replace', () => {
    let state = stream(running(), frame('Stale'));
    const observed = state;
    const saved = { id: String(segment), message_id: segment, msg_id: segment, conversation_id: conversationId,
      turn_id: root, position: 'left', type: 'text', content: { content: 'Stale snapshot' }, created_at: 100 } as TMessage;
    state = stream(state, frame('', { hidden: true }));
    expect(hydrateCompanionSessionBubble(state, [saved], observed).bubble).toBe('…');
    state = stream(state, frame('Correct', { replace: true }));
    expect(hydrateCompanionSessionBubble(state, [saved], observed).bubble).toBe('Correct');
    const beforeHidden = state;
    state = stream(state, frame('', { msg_id: secondSegment, hidden: true }));
    const missingSaved = { ...saved, msg_id: secondSegment, message_id: secondSegment } as TMessage;
    expect(hydrateCompanionSessionBubble(state, [missingSaved], beforeHidden).bubble).toBe('Correct');
  });
  test('canonical history rollback removes the last draft instead of retaining its old bubble', () => {
    const state = settleCompanionSessionBubble(stream(running(), frame('Draft')), root, 'Done');
    const row = { id: String(segment), message_id: segment, msg_id: segment, conversation_id: conversationId,
      turn_id: root, position: 'left', type: 'text', content: { content: '' }, created_at: 100 } as TMessage;
    expect(hydrateCompanionSessionBubble(state, [{ ...row, hidden: true }], state).bubble).toBe('');
    expect(hydrateCompanionSessionBubble(state, [row], state).bubble).toBe('');
  });
});
