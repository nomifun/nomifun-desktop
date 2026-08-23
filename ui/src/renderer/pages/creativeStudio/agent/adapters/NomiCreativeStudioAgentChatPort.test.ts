/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

import type {
  IConversationTurnCompletedEvent,
  IConversationTurnStartedEvent,
  IResponseMessage,
  ISendMessageResult,
} from '@/common/adapter/ipcBridge';
import {
  parseConversationId,
  parseMessageId,
  parseProviderId,
  type ConversationId,
} from '@/common/types/ids';

import type { CreativeStudioAgentTurnRequest } from '../chatPort';
import type { CreativeStudioAgentMessage } from '../types';
import {
  createNomiCreativeStudioAgentChatPort,
  NomiCreativeStudioAgentBindingError,
  NomiCreativeStudioAgentRuntimeError,
} from './NomiCreativeStudioAgentChatPort';
import { serializeCreativeStudioAgentHistory } from './history';
import type {
  NomiCreativeStudioAgentSessionBinding,
  NomiCreativeStudioAgentSessionResolution,
  NomiCreativeStudioAgentSessionResolutionInput,
  NomiCreativeStudioAgentSessionResolver,
  NomiCreativeStudioAgentTransport,
  NomiCreativeStudioConversationSnapshot,
} from './types';

const conversationId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000101');
const turnId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000102');
const userMessageId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000103');
const assistantMessageId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000104');
const idempotencyKey = '0190f5fe-7c00-7a00-8000-000000000106';
const displayPrompt = '基于当前画布继续创作';
const planningModelInput =
  '<creative-studio-planning>{"request":"基于当前画布继续创作","canvasRevision":7}</creative-studio-planning>';
const planningSkillIds = ['creative-studio-canvas', 'asset.inspect'] as const;
const model = {
  providerId: parseProviderId('0190f5fe-7c00-7a00-8000-000000000105'),
  model: 'nomi-chat-model',
};
const history: CreativeStudioAgentMessage[] = [
  {
    id: '0190f5fe-7c00-7a00-8000-000000000109',
    role: 'user',
    status: 'complete',
    text: '上一轮问题',
  },
  {
    id: '0190f5fe-7c00-7a00-8000-000000000110',
    role: 'assistant',
    status: 'complete',
    text: '上一轮回复',
  },
];
const recoveredHistory: CreativeStudioAgentMessage[] = [
  ...history,
  { id: userMessageId, role: 'user', status: 'complete', text: displayPrompt },
  { id: assistantMessageId, role: 'assistant', status: 'complete', text: '真实回复' },
];

const idleSnapshot = (
  overrides: Partial<NomiCreativeStudioConversationSnapshot> = {}
): NomiCreativeStudioConversationSnapshot => ({
  conversationId,
  model,
  authority: 'idle',
  ...overrides,
});

const acceptedReceipt = (
  overrides: Partial<ISendMessageResult> = {}
): ISendMessageResult => ({
  msg_id: userMessageId,
  replayed: false,
  completed: false,
  result_ok: null,
  result_text: null,
  result_error: null,
  result_error_code: null,
  result_error_retryable: null,
  ...overrides,
});

const turnStarted = (): IConversationTurnStartedEvent => ({
  conversation_id: conversationId,
  turn_id: turnId,
  status: 'running',
  phase: 'streaming',
  state: 'ai_generating',
  detail: '正在理解画布',
  can_send_message: false,
  runtime: {
    state: 'running',
    can_send_message: false,
    has_runtime: true,
    runtime_status: 'running',
    is_processing: true,
    pending_confirmations: 0,
    active_turn_id: turnId,
  },
});

const turnCompleted = (
  overrides: Partial<IConversationTurnCompletedEvent> = {}
): IConversationTurnCompletedEvent => ({
  conversation_id: conversationId,
  turn_id: turnId,
  status: 'finished',
  state: 'unknown',
  detail: '',
  can_send_message: true,
  runtime: {
    state: 'idle',
    can_send_message: true,
    has_runtime: false,
    runtime_status: 'finished',
    is_processing: false,
    pending_confirmations: 0,
  },
  workspace: '',
  model: { platform: 'test', name: 'Test', use_model: model.model },
  last_message: {
    message_id: assistantMessageId,
    type: 'text',
    content: '真实回复',
    status: 'finish',
    created_at: 1,
  },
  ...overrides,
});

class FakeTransport implements NomiCreativeStudioAgentTransport {
  readonly responseListeners = new Set<(event: IResponseMessage) => void>();
  readonly startedListeners = new Set<(event: IConversationTurnStartedEvent) => void>();
  readonly completedListeners = new Set<(event: IConversationTurnCompletedEvent) => void>();
  readonly reconnectListeners = new Set<() => void>();
  readonly inspectCalls: ConversationId[] = [];
  readonly sendCalls: Array<{
    conversationId: ConversationId;
    modelInput: string;
    skillIds: readonly string[];
    idempotencyKey: string;
  }> = [];
  readonly stopCalls: ConversationId[] = [];
  snapshots: NomiCreativeStudioConversationSnapshot[] = [idleSnapshot()];
  receipt = acceptedReceipt();
  onSend?: () => void;

  async inspect(id: ConversationId): Promise<NomiCreativeStudioConversationSnapshot> {
    this.inspectCalls.push(id);
    return this.snapshots.shift() ?? idleSnapshot();
  }

  async sendMessage(input: {
    conversationId: ConversationId;
    modelInput: string;
    skillIds: readonly string[];
    idempotencyKey: string;
  }): Promise<ISendMessageResult> {
    this.sendCalls.push(input);
    this.onSend?.();
    return this.receipt;
  }

  async stopAndConfirm(id: ConversationId): Promise<void> {
    this.stopCalls.push(id);
  }

  onResponse(listener: (event: IResponseMessage) => void): () => void {
    this.responseListeners.add(listener);
    return () => this.responseListeners.delete(listener);
  }

  onTurnStarted(listener: (event: IConversationTurnStartedEvent) => void): () => void {
    this.startedListeners.add(listener);
    return () => this.startedListeners.delete(listener);
  }

  onTurnCompleted(listener: (event: IConversationTurnCompletedEvent) => void): () => void {
    this.completedListeners.add(listener);
    return () => this.completedListeners.delete(listener);
  }

  onReconnected(listener: () => void): () => void {
    this.reconnectListeners.add(listener);
    return () => this.reconnectListeners.delete(listener);
  }

  emitResponse(event: IResponseMessage): void {
    for (const listener of this.responseListeners) listener(event);
  }

  emitStarted(event: IConversationTurnStartedEvent): void {
    for (const listener of this.startedListeners) listener(event);
  }

  emitCompleted(event: IConversationTurnCompletedEvent): void {
    for (const listener of this.completedListeners) listener(event);
  }
}

interface ResolverOverrides {
  binding?: Partial<NomiCreativeStudioAgentSessionBinding>;
  appliedProposalMessageIds?: NomiCreativeStudioAgentSessionResolution['appliedProposalMessageIds'];
  history?:
    | readonly CreativeStudioAgentMessage[]
    | ((call: number, input: NomiCreativeStudioAgentSessionResolutionInput) => readonly CreativeStudioAgentMessage[]);
  created?: boolean;
}

const matchingResolver = (overrides: ResolverOverrides = {}): NomiCreativeStudioAgentSessionResolver => {
  let calls = 0;
  return async (input): Promise<NomiCreativeStudioAgentSessionResolution> => {
    const historyOverride = overrides.history;
    const authoritativeHistory =
      typeof historyOverride === 'function'
        ? historyOverride(calls, input)
        : (historyOverride ?? history);
    calls += 1;
    return {
      binding: {
        ownership: 'creative-studio-exclusive',
        canvasId: input.canvasId,
        sessionId: input.sessionId,
        conversationId,
        model: input.model,
        historyKey: serializeCreativeStudioAgentHistory(authoritativeHistory),
        ...overrides.binding,
      },
      history: authoritativeHistory,
      appliedProposalMessageIds: overrides.appliedProposalMessageIds ?? [],
      created: overrides.created ?? false,
    };
  };
};

const request = (
  signal: AbortSignal,
  overrides: Partial<Omit<CreativeStudioAgentTurnRequest, 'signal'>> = {}
): CreativeStudioAgentTurnRequest => ({
  canvasId: '0190f5fe-7c00-7a00-8000-000000000107',
  sessionId: '0190f5fe-7c00-7a00-8000-000000000108',
  idempotencyKey,
  prompt: displayPrompt,
  modelInput: planningModelInput,
  skillIds: planningSkillIds,
  model,
  history,
  ...overrides,
  signal,
});

const collect = async <T>(
  source: AsyncIterable<T> | Promise<AsyncIterable<T>>
): Promise<T[]> => {
  const iterable = await source;
  const values: T[] = [];
  for await (const value of iterable) values.push(value);
  return values;
};

describe('NomiCreativeStudioAgentChatPort', () => {
  test('maps real REST admission plus exact WS turn lifecycle into port events', async () => {
    const transport = new FakeTransport();
    const mutableSkillIds = [...planningSkillIds];
    transport.snapshots = [
      idleSnapshot(),
      idleSnapshot({ authority: 'processing', activeTurnId: turnId }),
      idleSnapshot({ authority: 'processing', activeTurnId: turnId }),
    ];
    transport.onSend = () => {
      queueMicrotask(() => {
        transport.emitStarted(turnStarted());
        transport.emitResponse({
          type: 'content',
          data: '真实',
          msg_id: assistantMessageId,
          turn_id: turnId,
          conversation_id: conversationId,
        });
        transport.emitResponse({
          type: 'content',
          data: '回复',
          msg_id: assistantMessageId,
          turn_id: turnId,
          conversation_id: conversationId,
        });
        transport.emitCompleted(turnCompleted());
      });
    };
    const port = createNomiCreativeStudioAgentChatPort({
      resolveSession: matchingResolver({
        history: (call) => (call === 0 ? history : recoveredHistory),
      }),
      transport,
      turnStartTimeoutMs: 100,
    });

    const turnRequest = request(new AbortController().signal, { skillIds: mutableSkillIds });
    const events = await collect(port.runTurn(turnRequest));
    mutableSkillIds.reverse();

    expect(events).toEqual([
      { type: 'activity', label: '正在理解画布' },
      { type: 'assistant-delta', delta: '真实' },
      { type: 'assistant-delta', delta: '回复' },
      { type: 'history-reconciled', history: recoveredHistory },
      { type: 'completed', assistantMessageId },
    ]);
    expect(transport.sendCalls).toEqual([
      {
        conversationId,
        modelInput: planningModelInput,
        skillIds: planningSkillIds,
        idempotencyKey,
      },
    ]);
    expect(transport.sendCalls[0]?.skillIds).not.toBe(turnRequest.skillIds);
    expect(transport.responseListeners.size).toBe(0);
    expect(transport.completedListeners.size).toBe(0);
  });

  test('rejects invalid durable planning envelopes before touching the transport', async () => {
    const cases: Array<{
      label: string;
      overrides: Partial<Omit<CreativeStudioAgentTurnRequest, 'signal'>>;
    }> = [
      { label: 'blank model input', overrides: { modelInput: '' } },
      { label: 'padded model input', overrides: { modelInput: ' padded' } },
      {
        label: 'oversized model input',
        overrides: { modelInput: 'x'.repeat(262_145) },
      },
      {
        label: 'too many skills',
        overrides: {
          skillIds: Array.from({ length: 9 }, (_, index) => `skill-${index}`),
        },
      },
      { label: 'non-ascii skill', overrides: { skillIds: ['canvas.检查'] } },
      { label: 'duplicate skills', overrides: { skillIds: ['canvas.read', 'canvas.read'] } },
    ];

    for (const invalid of cases) {
      const transport = new FakeTransport();
      let resolverCalls = 0;
      const resolveSession = matchingResolver();
      const port = createNomiCreativeStudioAgentChatPort({
        resolveSession: async (input) => {
          resolverCalls += 1;
          return resolveSession(input);
        },
        transport,
      });

      const error = await collect(
        port.runTurn(request(new AbortController().signal, invalid.overrides))
      ).catch((caught: unknown) => caught);

      expect({ label: invalid.label, isTypeError: error instanceof TypeError }).toEqual({
        label: invalid.label,
        isTypeError: true,
      });
      expect(resolverCalls).toBe(0);
      expect(transport.inspectCalls).toEqual([]);
      expect(transport.sendCalls).toEqual([]);
    }
  });

  test('fails before send when Canvas/session/model/history binding is not exact', async () => {
    const transport = new FakeTransport();
    const port = createNomiCreativeStudioAgentChatPort({
      resolveSession: matchingResolver({ binding: { historyKey: 'different-history' } }),
      transport,
    });
    let error: unknown;

    try {
      await collect(port.runTurn(request(new AbortController().signal)));
    } catch (caught) {
      error = caught;
    }

    expect(error instanceof NomiCreativeStudioAgentBindingError).toBe(true);
    expect(transport.inspectCalls).toEqual([]);
    expect(transport.sendCalls).toEqual([]);
  });

  test('rechecks the real conversation model and idle authority at send time', async () => {
    const transport = new FakeTransport();
    transport.snapshots = [
      idleSnapshot({
        model: {
          providerId: parseProviderId('0190f5fe-7c00-7a00-8000-000000000108'),
          model: model.model,
        },
      }),
    ];
    const port = createNomiCreativeStudioAgentChatPort({
      resolveSession: matchingResolver({
        history: (call) => (call === 0 ? history : recoveredHistory),
      }),
      transport,
      recoveryPollMs: 25,
      turnStartTimeoutMs: 100,
    });
    let error: unknown;

    try {
      await collect(port.runTurn(request(new AbortController().signal)));
    } catch (caught) {
      error = caught;
    }

    expect(error instanceof NomiCreativeStudioAgentBindingError).toBe(true);
    expect(transport.sendCalls).toEqual([]);
  });

  test('uses a real durable replay receipt without inventing an assistant message id', async () => {
    const transport = new FakeTransport();
    transport.receipt = acceptedReceipt({
      replayed: true,
      completed: true,
      result_ok: true,
      result_text: '后端持久化回复',
    });
    const port = createNomiCreativeStudioAgentChatPort({
      resolveSession: matchingResolver({
        history: (call) => (call === 0 ? history : recoveredHistory),
      }),
      transport,
      recoveryPollMs: 25,
      turnStartTimeoutMs: 100,
    });

    const events = await collect(port.runTurn(request(new AbortController().signal)));

    expect(events).toEqual([
      { type: 'history-reconciled', history: recoveredHistory },
      { type: 'completed', assistantMessageId },
    ]);
    expect(transport.sendCalls).toEqual([
      {
        conversationId,
        modelInput: planningModelInput,
        skillIds: planningSkillIds,
        idempotencyKey,
      },
    ]);
  });

  test('reconciles a response-loss completion before transport without submitting twice', async () => {
    const transport = new FakeTransport();
    const port = createNomiCreativeStudioAgentChatPort({
      resolveSession: matchingResolver({ history: recoveredHistory }),
      transport,
    });

    const events = await collect(port.runTurn(request(new AbortController().signal)));

    expect(events).toEqual([
      { type: 'history-reconciled', history: recoveredHistory },
      { type: 'completed', assistantMessageId },
    ]);
    expect(transport.inspectCalls).toEqual([]);
    expect(transport.sendCalls).toEqual([]);
  });

  test('recovers an active durable replay after missed WebSocket terminal events', async () => {
    const transport = new FakeTransport();
    transport.receipt = acceptedReceipt({ replayed: true });
    transport.snapshots = [
      idleSnapshot({ authority: 'processing', activeTurnId: turnId }),
      idleSnapshot({ authority: 'processing', activeTurnId: turnId }),
      idleSnapshot(),
    ];
    const port = createNomiCreativeStudioAgentChatPort({
      resolveSession: matchingResolver({
        history: (call) => (call === 0 ? history : recoveredHistory),
      }),
      transport,
      recoveryPollMs: 25,
      turnStartTimeoutMs: 100,
    });

    const events = await collect(port.runTurn(request(new AbortController().signal)));

    expect(events).toEqual([
      { type: 'history-reconciled', history: recoveredHistory },
      { type: 'completed', assistantMessageId },
    ]);
    expect(transport.sendCalls).toEqual([
      {
        conversationId,
        modelInput: planningModelInput,
        skillIds: planningSkillIds,
        idempotencyKey,
      },
    ]);
  });

  test('never reports success when a terminal receipt has no durable message pair', async () => {
    const transport = new FakeTransport();
    transport.receipt = acceptedReceipt({
      replayed: true,
      completed: true,
      result_ok: true,
      result_text: 'only a transient receipt result',
    });
    const port = createNomiCreativeStudioAgentChatPort({
      resolveSession: matchingResolver(),
      transport,
      recoveryPollMs: 25,
      turnStartTimeoutMs: 50,
    });

    const error = await collect(port.runTurn(request(new AbortController().signal))).catch(
      (caught: unknown) => caught
    );

    expect(error instanceof NomiCreativeStudioAgentRuntimeError).toBe(true);
    expect(error instanceof NomiCreativeStudioAgentRuntimeError ? error.code : '').toBe(
      'HISTORY_RECONCILIATION_TIMEOUT'
    );
  });

  test('AbortSignal requests backend stop and waits for its confirmation boundary', async () => {
    const transport = new FakeTransport();
    transport.snapshots = [
      idleSnapshot(),
      idleSnapshot({ authority: 'processing', activeTurnId: turnId }),
    ];
    const controller = new AbortController();
    const port = createNomiCreativeStudioAgentChatPort({
      resolveSession: matchingResolver(),
      transport,
      turnStartTimeoutMs: 1_000,
    });
    let error: unknown;
    const running = collect(port.runTurn(request(controller.signal))).catch((caught) => {
      error = caught;
      return [];
    });

    for (let attempt = 0; attempt < 10 && transport.sendCalls.length === 0; attempt += 1) {
      await Promise.resolve();
    }
    expect(transport.sendCalls.length).toBe(1);
    controller.abort();
    await running;

    expect(error instanceof Error ? error.name : '').toBe('AbortError');
    expect(transport.stopCalls).toEqual([conversationId]);
    expect(transport.responseListeners.size).toBe(0);
  });

  test('stop-confirms a correlated stream error before exposing it as terminal', async () => {
    const transport = new FakeTransport();
    transport.snapshots = [
      idleSnapshot(),
      idleSnapshot({ authority: 'processing', activeTurnId: turnId }),
      idleSnapshot({ authority: 'processing', activeTurnId: turnId }),
    ];
    transport.onSend = () => {
      queueMicrotask(() => {
        transport.emitStarted(turnStarted());
        transport.emitResponse({
          type: 'error',
          data: { message: 'rate limited' },
          msg_id: assistantMessageId,
          turn_id: turnId,
          conversation_id: conversationId,
        });
      });
    };
    const port = createNomiCreativeStudioAgentChatPort({
      resolveSession: matchingResolver(),
      transport,
      turnStartTimeoutMs: 100,
    });

    const events = await collect(port.runTurn(request(new AbortController().signal)));

    expect(events[0]?.type).toBe('activity');
    expect(events[1]?.type).toBe('failed');
    expect(transport.stopCalls).toEqual([conversationId]);
    expect(transport.responseListeners.size).toBe(0);
  });

  test('fails closed when a replacement cannot be represented as append-only deltas', async () => {
    const transport = new FakeTransport();
    transport.snapshots = [
      idleSnapshot(),
      idleSnapshot({ authority: 'processing', activeTurnId: turnId }),
      idleSnapshot({ authority: 'processing', activeTurnId: turnId }),
    ];
    transport.onSend = () => {
      queueMicrotask(() => {
        transport.emitStarted(turnStarted());
        transport.emitResponse({
          type: 'content',
          data: 'first',
          msg_id: assistantMessageId,
          turn_id: turnId,
          conversation_id: conversationId,
        });
        transport.emitResponse({
          type: 'content',
          data: 'different',
          replace: true,
          msg_id: assistantMessageId,
          turn_id: turnId,
          conversation_id: conversationId,
        });
      });
    };
    const port = createNomiCreativeStudioAgentChatPort({
      resolveSession: matchingResolver(),
      transport,
      turnStartTimeoutMs: 100,
    });
    let error: unknown;

    try {
      await collect(port.runTurn(request(new AbortController().signal)));
    } catch (caught) {
      error = caught;
    }

    expect(error instanceof NomiCreativeStudioAgentRuntimeError).toBe(true);
    expect(error instanceof NomiCreativeStudioAgentRuntimeError ? error.code : '').toBe(
      'NON_APPEND_REPLACEMENT'
    );
    expect(transport.stopCalls).toEqual([conversationId]);
  });
});

describe('Nomi adapter boundaries', () => {
  test('serializes the complete controlled history without dropping failure metadata', () => {
    const first = serializeCreativeStudioAgentHistory([
      ...history,
      {
        id: 'failed-1',
        role: 'assistant',
        status: 'failed',
        text: '',
        errorMessage: '真实错误',
      },
    ]);
    const second = serializeCreativeStudioAgentHistory(history);

    expect(first.includes('真实错误')).toBe(true);
    expect(first === second).toBe(false);
  });

  test('depends on service/IPC seams, never conversation components or route state', () => {
    const adapter = readFileSync(
      new URL('./NomiCreativeStudioAgentChatPort.ts', import.meta.url),
      'utf8'
    );
    const transport = readFileSync(new URL('./nomiTransport.ts', import.meta.url), 'utf8');

    expect(transport.includes('conversation.sendMessage.invoke')).toBe(true);
    expect(transport.includes('conversation.responseStream.on')).toBe(true);
    expect(transport.includes('conversation.turnStarted.on')).toBe(true);
    expect(transport.includes('conversation.turnCompleted.on')).toBe(true);
    expect(transport.includes('stopConversationAndConfirmRelease')).toBe(true);
    expect(transport.includes('input: modelInput')).toBe(true);
    expect(transport.includes('inject_skills: [...skillIds]')).toBe(true);
    expect(transport.includes('input: prompt')).toBe(false);
    expect(adapter.includes('useNavigate')).toBe(false);
    expect(adapter.includes('ChatConversation')).toBe(false);
    expect(adapter.includes('NomiSendBox')).toBe(false);
  });
});
