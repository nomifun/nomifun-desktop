/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IResponseMessage } from '@/common/adapter/ipcBridge';
import { extractResponseTextChunk } from '@/common/chat/displayText';
import type { TMessage } from '@/common/chat/chatLib';
import type { ConversationId, MessageId } from '@/common/types/ids';

export type CompanionBubblePhase = 'idle' | 'accepted' | 'running' | 'paused' | 'settled';

/** Ephemeral presentation of one canonical Turn; it never owns execution. */
export interface CompanionSessionBubbleState {
  conversationId: ConversationId | null;
  turnId: MessageId | null;
  phase: CompanionBubblePhase;
  bubble: string;
  loading: boolean;
  dismissed: boolean;
  interrupted: boolean;
  remoteHeader: { platform: string; inbound: string } | null;
  segments: ReadonlyMap<MessageId, string>;
  segmentRevisions: ReadonlyMap<MessageId, number>;
  order: readonly MessageId[];
  hint: string;
}

export const emptyCompanionSessionBubble = (conversationId: ConversationId | null): CompanionSessionBubbleState => ({
  conversationId, turnId: null, phase: 'idle', bubble: '', loading: false,
  dismissed: false, interrupted: false, remoteHeader: null,
  segments: new Map(), segmentRevisions: new Map(), order: [], hint: '',
});

const joinedText = (state: CompanionSessionBubbleState): string =>
  state.order.map(id => state.segments.get(id) ?? '').filter(Boolean).join('\n\n').trim();

export const beginCompanionSessionBubble = (
  state: CompanionSessionBubbleState, turnId: MessageId, phase: 'accepted' | 'running'
): CompanionSessionBubbleState => {
  if (state.turnId === turnId) {
    if (state.phase === 'settled' || ((state.phase === 'running' || state.phase === 'paused') && phase === 'accepted')) return state;
    return state.phase === phase ? state : { ...state, phase };
  }
  return { ...emptyCompanionSessionBubble(state.conversationId), turnId, phase, bubble: '…', loading: true };
};

/** A history read fills missing segments without replacing text streamed during the read. */
export const hydrateCompanionSessionBubble = (
  state: CompanionSessionBubbleState,
  messages: readonly TMessage[],
  observed: Pick<CompanionSessionBubbleState, 'segments' | 'segmentRevisions'>
): CompanionSessionBubbleState => {
  if (!state.turnId || state.dismissed || state.interrupted) return state;
  const owned = messages.filter(message => message.conversation_id === state.conversationId
    && message.turn_id === state.turnId && message.type === 'text' && message.position === 'left')
    .sort((left, right) => (left.created_at ?? 0) - (right.created_at ?? 0));
  const segments = new Map(state.segments);
  const historyOrder: MessageId[] = [];
  for (const message of owned) {
    const id = message.msg_id ?? message.message_id;
    if (!id) continue;
    if (state.segmentRevisions.get(id) !== observed.segmentRevisions.get(id)) continue;
    if (message.hidden) {
      segments.delete(id);
      continue;
    }
    historyOrder.push(id);
    const live = segments.get(id);
    const saved = extractResponseTextChunk(message.content);
    if (live !== observed.segments.get(id) || (state.phase !== 'settled' && live && live.length > saved.length)) continue;
    segments.set(id, saved);
  }
  const order = state.order.filter(id => segments.has(id));
  historyOrder.forEach((id, index) => {
    if (order.includes(id)) return;
    const following = historyOrder.slice(index + 1).find(candidate => order.includes(candidate));
    if (following) order.splice(order.indexOf(following), 0, id);
    else order.push(id);
  });
  const next = { ...state, segments, order };
  const text = joinedText(next);
  if (!owned.length) return state;
  return { ...next, bubble: text || state.hint || (state.phase === 'settled' ? '' : '…'),
    loading: !text && !state.hint && state.phase !== 'settled' };
};

export const streamCompanionSessionBubble = (
  state: CompanionSessionBubbleState,
  message: IResponseMessage,
  toolHint: string,
  errorHint: string
): CompanionSessionBubbleState => {
  if (message.conversation_id !== state.conversationId || !message.turn_id || message.turn_id !== state.turnId
    || message.stream_complete || state.dismissed || state.interrupted) return state;
  if (message.hidden) {
    const segments = new Map(state.segments);
    segments.delete(message.msg_id);
    const segmentRevisions = new Map(state.segmentRevisions);
    segmentRevisions.set(message.msg_id, (segmentRevisions.get(message.msg_id) ?? 0) + 1);
    const next = { ...state, segments, segmentRevisions, order: state.order.filter(id => id !== message.msg_id) };
    const text = joinedText(next);
    return { ...next, bubble: text || state.hint || (state.phase === 'settled' ? '' : '…'), loading: !text && !state.hint };
  }
  if (message.type === 'content' || message.type === 'text') {
    const chunk = extractResponseTextChunk(message.data);
    if ((!chunk && !message.replace) || !message.msg_id) return state;
    const segments = new Map(state.segments);
    const segmentRevisions = new Map(state.segmentRevisions);
    segmentRevisions.set(message.msg_id, (segmentRevisions.get(message.msg_id) ?? 0) + 1);
    const order = segments.has(message.msg_id) ? state.order : [...state.order, message.msg_id];
    segments.set(message.msg_id, message.replace ? chunk : (segments.get(message.msg_id) ?? '') + chunk);
    const next = { ...state, segments, segmentRevisions, order };
    const text = joinedText(next);
    return { ...next, bubble: text || (state.phase === 'settled' ? '' : '…'), loading: !text && state.phase !== 'settled' };
  }
  if (state.phase === 'settled') return state;
  if (message.type === 'tool_call' || message.type === 'tool_group' || message.type === 'error') {
    const hint = message.type === 'error' ? errorHint : toolHint;
    return { ...state, hint, bubble: joinedText(state) || hint, loading: false };
  }
  // Thinking and output_discarded are activity, not a new Turn or a deletion.
  // Rollbacks arrive as authoritative replace/hidden updates for exact segments.
  return state;
};

export const settleCompanionSessionBubble = (
  state: CompanionSessionBubbleState, turnId: MessageId, fallback: string, paused = false
): CompanionSessionBubbleState => {
  if (state.turnId !== turnId || state.phase === 'settled') return state;
  return { ...state, phase: paused ? 'paused' : 'settled', loading: false,
    bubble: state.dismissed ? '' : joinedText(state) || state.hint || fallback };
};

export const clearCompanionSessionBubble = (
  state: CompanionSessionBubbleState, turnId: MessageId | null
): CompanionSessionBubbleState =>
  state.turnId === turnId && state.phase === 'settled' ? { ...state, bubble: '', dismissed: true } : state;
