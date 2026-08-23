/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'vitest';

import { parseConversationId, parseMessageId, parseProviderId } from '@/common/types/ids';
import { serializeCreativeStudioAgentHistory } from '../adapters';
import type {
  NomiCreativeStudioAgentSessionBinding,
  NomiCreativeStudioAgentSessionResolution,
  NomiCreativeStudioAgentSessionResolutionInput,
} from '../adapters';
import type { CreativeStudioAgentMessage } from '../types';
import {
  CreativeStudioAgentSessionBackendUnavailableError,
  CreativeStudioAgentSessionController,
  CreativeStudioAgentSessionResolutionError,
  createFailClosedCreativeStudioAgentSessionPort,
  type CreativeStudioAgentSessionPersistencePort,
  type CreativeStudioAgentSessionPersistenceRequest,
} from '.';

const providerId = parseProviderId('0190f5fe-7c00-7a00-8000-000000000401');
const conversationA = parseConversationId('0190f5fe-7c00-7a00-8000-000000000501');
const conversationB = parseConversationId('0190f5fe-7c00-7a00-8000-000000000502');
const pendingKey = '0190f5fe-7c00-7a00-8000-000000000503';
const replacementPendingKey = '0190f5fe-7c00-7a00-8000-000000000504';
const model = { providerId, model: 'nomi-chat' } as const;
const userMessageId = '0190f5fe-7c00-7a00-8000-000000000505';
const assistantMessageId = '0190f5fe-7c00-7a00-8000-000000000506';
const history: readonly CreativeStudioAgentMessage[] = [
  { id: userMessageId, role: 'user', status: 'complete', text: '制作一张海报' },
  { id: assistantMessageId, role: 'assistant', status: 'complete', text: '我先整理画布。' },
];

const input = (
  overrides: Partial<NomiCreativeStudioAgentSessionResolutionInput> = {}
): NomiCreativeStudioAgentSessionResolutionInput => {
  return {
    canvasId: 'canvas-a',
    sessionId: 'session-a',
    model,
    signal: new AbortController().signal,
    ...overrides,
    pendingTurnIdempotencyKey: overrides.pendingTurnIdempotencyKey ?? null,
  };
};

const binding = (
  request: CreativeStudioAgentSessionPersistenceRequest,
  conversationId = conversationA,
  authoritativeHistory: readonly CreativeStudioAgentMessage[] = history
): NomiCreativeStudioAgentSessionBinding => ({
  ownership: 'creative-studio-exclusive',
  canvasId: request.canvasId,
  sessionId: request.sessionId,
  conversationId,
  model: request.model,
  historyKey: serializeCreativeStudioAgentHistory(authoritativeHistory),
});

const resolution = (
  request: CreativeStudioAgentSessionPersistenceRequest,
  conversationId = conversationA,
  overrides: Partial<NomiCreativeStudioAgentSessionResolution> = {}
): NomiCreativeStudioAgentSessionResolution => {
  const authoritativeHistory = overrides.history ?? history;
  return {
    binding: {
      ...binding(request, conversationId, authoritativeHistory),
      historyKey: serializeCreativeStudioAgentHistory(authoritativeHistory),
      ...overrides.binding,
    },
    history: authoritativeHistory,
    appliedProposalMessageIds: overrides.appliedProposalMessageIds ?? [],
    created: overrides.created ?? false,
  };
};

class DeferredPort implements CreativeStudioAgentSessionPersistencePort {
  readonly calls: CreativeStudioAgentSessionPersistenceRequest[] = [];
  private resolvePending?: (value: NomiCreativeStudioAgentSessionResolution) => void;

  resolveOrCreateExclusive(
    request: CreativeStudioAgentSessionPersistenceRequest
  ): Promise<NomiCreativeStudioAgentSessionResolution> {
    this.calls.push(request);
    return new Promise((resolve) => {
      this.resolvePending = resolve;
    });
  }

  release(): void {
    const request = this.calls.at(-1);
    if (!request || !this.resolvePending) throw new Error('no pending resolution');
    this.resolvePending(resolution(request));
  }
}

describe('CreativeStudioAgentSessionController', () => {
  test('coalesces concurrent resolution for one Canvas session', async () => {
    const port = new DeferredPort();
    const controller = new CreativeStudioAgentSessionController(port);

    const first = controller.resolve(input());
    const second = controller.resolve(input());
    await Promise.resolve();

    expect(port.calls).toHaveLength(1);
    port.release();
    const resolved = await Promise.all([first, second]);
    expect(resolved.map((item) => item.binding.conversationId)).toEqual([
      conversationA,
      conversationA,
    ]);
  });

  test('survives a StrictMode-style aborted waiter without cancelling its replacement', async () => {
    const port = new DeferredPort();
    const controller = new CreativeStudioAgentSessionController(port);
    const firstAbort = new AbortController();

    const first = controller.resolve(input({ signal: firstAbort.signal }));
    await Promise.resolve();
    firstAbort.abort();
    const replacement = controller.resolve(input());

    const firstFailure = await first.catch((error: unknown) => error);
    expect(firstFailure instanceof Error).toBe(true);
    expect((firstFailure as Error).name).toBe('AbortError');
    expect(port.calls).toHaveLength(1);
    port.release();
    expect((await replacement).binding.conversationId).toBe(conversationA);
  });

  test('restores through the durable port after controller recreation', async () => {
    const persisted = new Map<string, NomiCreativeStudioAgentSessionBinding>();
    let calls = 0;
    const port: CreativeStudioAgentSessionPersistencePort = {
      async resolveOrCreateExclusive(request) {
        calls += 1;
        const key = `${request.canvasId}/${request.sessionId}`;
        const restored = persisted.get(key) ?? binding(request);
        persisted.set(key, restored);
        return resolution(request, restored.conversationId, { binding: restored });
      },
    };

    const first = await new CreativeStudioAgentSessionController(port).resolve(input());
    const restored = await new CreativeStudioAgentSessionController(port).resolve(input());

    expect(calls).toBe(2);
    expect(restored).toEqual(first);
  });

  test('does not coalesce sessions from different Canvases', async () => {
    const calls: CreativeStudioAgentSessionPersistenceRequest[] = [];
    const port: CreativeStudioAgentSessionPersistencePort = {
      async resolveOrCreateExclusive(request) {
        calls.push(request);
        return resolution(
          request,
          request.canvasId === 'canvas-a' ? conversationA : conversationB
        );
      },
    };
    const controller = new CreativeStudioAgentSessionController(port);

    const [first, second] = await Promise.all([
      controller.resolve(input()),
      controller.resolve(input({ canvasId: 'canvas-b' })),
    ]);

    expect(calls.map((request) => request.canvasId).sort()).toEqual(['canvas-a', 'canvas-b']);
    expect(first.binding.conversationId).toBe(conversationA);
    expect(second.binding.conversationId).toBe(conversationB);
  });

  test('does not coalesce different durable pending-turn proofs for one session', async () => {
    const calls: CreativeStudioAgentSessionPersistenceRequest[] = [];
    const port: CreativeStudioAgentSessionPersistencePort = {
      async resolveOrCreateExclusive(request) {
        calls.push(request);
        return resolution(request);
      },
    };
    const controller = new CreativeStudioAgentSessionController(port);

    await Promise.all([
      controller.resolve(input({ pendingTurnIdempotencyKey: pendingKey })),
      controller.resolve(input({ pendingTurnIdempotencyKey: replacementPendingKey })),
    ]);

    expect(calls.map((request) => request.pendingTurnIdempotencyKey).sort()).toEqual([
      pendingKey,
      replacementPendingKey,
    ]);
  });

  test('rejects a cross-Canvas binding returned by the persistence port', async () => {
    const port: CreativeStudioAgentSessionPersistencePort = {
      async resolveOrCreateExclusive(request) {
        return resolution(request, conversationA, {
          binding: { ...binding(request), canvasId: 'canvas-b' },
        });
      },
    };

    const failure = await new CreativeStudioAgentSessionController(port)
      .resolve(input())
      .catch((error: unknown) => error);
    expect(failure instanceof CreativeStudioAgentSessionResolutionError).toBe(true);
    expect((failure as CreativeStudioAgentSessionResolutionError).code).toBe(
      'PORT_CONTRACT_VIOLATION'
    );
  });

  test('evicts a failed operation so an explicit retry reaches persistence again', async () => {
    let attempts = 0;
    const port: CreativeStudioAgentSessionPersistencePort = {
      async resolveOrCreateExclusive(request) {
        attempts += 1;
        if (attempts === 1) throw new Error('temporary backend failure');
        return resolution(request);
      },
    };
    const controller = new CreativeStudioAgentSessionController(port);

    const firstFailure = await controller.resolve(input()).catch((error: unknown) => error);
    expect(firstFailure instanceof Error).toBe(true);
    expect((firstFailure as Error).message).toBe('temporary backend failure');
    expect((await controller.resolve(input())).binding.conversationId).toBe(conversationA);
    expect(attempts).toBe(2);
  });

  test('loads the complete server-authoritative history behind a durable pending fence', async () => {
    const recoveredHistory: readonly CreativeStudioAgentMessage[] = [
      ...history,
      { id: 'message-3', role: 'user', status: 'complete', text: '继续制作' },
      { id: 'message-4', role: 'assistant', status: 'complete', text: '已完成' },
    ];
    const port: CreativeStudioAgentSessionPersistencePort = {
      async resolveOrCreateExclusive(request) {
        return resolution(request, conversationA, { history: recoveredHistory });
      },
    };

    const recovered = await new CreativeStudioAgentSessionController(port).resolve(
      input({ pendingTurnIdempotencyKey: pendingKey })
    );
    expect(recovered.history).toEqual(recoveredHistory);
  });

  test('fails when persistence returns history that does not match its binding proof', async () => {
    const port: CreativeStudioAgentSessionPersistencePort = {
      async resolveOrCreateExclusive(request) {
        return resolution(request, conversationA, {
          binding: { ...binding(request), historyKey: 'stale' },
        });
      },
    };

    const failure = await new CreativeStudioAgentSessionController(port)
      .resolve(input())
      .catch((error: unknown) => error);
    expect(failure instanceof CreativeStudioAgentSessionResolutionError).toBe(true);
    expect((failure as CreativeStudioAgentSessionResolutionError).code).toBe(
      'PORT_CONTRACT_VIOLATION'
    );
  });

  test('rejects applied proposal receipts outside the authoritative assistant history', async () => {
    const port: CreativeStudioAgentSessionPersistencePort = {
      async resolveOrCreateExclusive(request) {
        return resolution(request, conversationA, {
          appliedProposalMessageIds: [parseMessageId(userMessageId)],
        });
      },
    };

    const failure = await new CreativeStudioAgentSessionController(port)
      .resolve(input())
      .catch((error: unknown) => error);
    expect(failure instanceof CreativeStudioAgentSessionResolutionError).toBe(true);
    expect((failure as CreativeStudioAgentSessionResolutionError).code).toBe(
      'PORT_CONTRACT_VIOLATION'
    );
  });

  test('the current production boundary fails closed with an actionable backend contract', async () => {
    const controller = new CreativeStudioAgentSessionController(
      createFailClosedCreativeStudioAgentSessionPort()
    );

    const failure = await controller.resolve(input()).catch((error: unknown) => error);
    expect(failure instanceof CreativeStudioAgentSessionBackendUnavailableError).toBe(true);
    const unavailable = failure as CreativeStudioAgentSessionBackendUnavailableError;
    expect(unavailable.code).toBe('ATOMIC_EXCLUSIVE_SESSION_BINDING_UNAVAILABLE');
    expect(
      unavailable.requiredContract.some((requirement) =>
        requirement.includes('database uniqueness constraint')
      )
    ).toBe(true);
    expect(
      unavailable.requiredContract.some((requirement) =>
        requirement.includes('server-owned exclusive-ownership marker')
      )
    ).toBe(true);
  });
});
