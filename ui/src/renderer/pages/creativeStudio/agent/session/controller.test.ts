/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'vitest';

import { parseConversationId, parseProviderId } from '@/common/types/ids';
import { serializeCreativeStudioAgentHistory } from '../adapters';
import type {
  NomiCreativeStudioAgentSessionBinding,
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
const model = { providerId, model: 'nomi-chat' } as const;
const history: readonly CreativeStudioAgentMessage[] = [
  { id: 'message-1', role: 'user', status: 'complete', text: '制作一张海报' },
  { id: 'message-2', role: 'assistant', status: 'complete', text: '我先整理画布。' },
];

const input = (
  overrides: Partial<NomiCreativeStudioAgentSessionResolutionInput> = {}
): NomiCreativeStudioAgentSessionResolutionInput => {
  const selectedHistory = overrides.history ?? history;
  return {
    projectId: 'project-a',
    sessionId: 'session-a',
    model,
    history: selectedHistory,
    historyKey: serializeCreativeStudioAgentHistory(selectedHistory),
    signal: new AbortController().signal,
    ...overrides,
  };
};

const binding = (
  request: CreativeStudioAgentSessionPersistenceRequest,
  conversationId = conversationA
): NomiCreativeStudioAgentSessionBinding => ({
  ownership: 'creative-studio-exclusive',
  projectId: request.projectId,
  sessionId: request.sessionId,
  conversationId,
  model: request.model,
  historyKey: request.historyKey,
});

class DeferredPort implements CreativeStudioAgentSessionPersistencePort {
  readonly calls: CreativeStudioAgentSessionPersistenceRequest[] = [];
  private resolvePending?: (value: NomiCreativeStudioAgentSessionBinding) => void;

  resolveOrCreateExclusive(
    request: CreativeStudioAgentSessionPersistenceRequest
  ): Promise<NomiCreativeStudioAgentSessionBinding> {
    this.calls.push(request);
    return new Promise((resolve) => {
      this.resolvePending = resolve;
    });
  }

  release(): void {
    const request = this.calls.at(-1);
    if (!request || !this.resolvePending) throw new Error('no pending resolution');
    this.resolvePending(binding(request));
  }
}

describe('CreativeStudioAgentSessionController', () => {
  test('coalesces concurrent resolution for one project session', async () => {
    const port = new DeferredPort();
    const controller = new CreativeStudioAgentSessionController(port);

    const first = controller.resolve(input());
    const second = controller.resolve(input());
    await Promise.resolve();

    expect(port.calls).toHaveLength(1);
    port.release();
    const resolved = await Promise.all([first, second]);
    expect(resolved.map((item) => item.conversationId)).toEqual([
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
    expect((await replacement).conversationId).toBe(conversationA);
  });

  test('restores through the durable port after controller recreation', async () => {
    const persisted = new Map<string, NomiCreativeStudioAgentSessionBinding>();
    let calls = 0;
    const port: CreativeStudioAgentSessionPersistencePort = {
      async resolveOrCreateExclusive(request) {
        calls += 1;
        const key = `${request.projectId}/${request.sessionId}`;
        const restored = persisted.get(key) ?? binding(request);
        persisted.set(key, restored);
        return restored;
      },
    };

    const first = await new CreativeStudioAgentSessionController(port).resolve(input());
    const restored = await new CreativeStudioAgentSessionController(port).resolve(input());

    expect(calls).toBe(2);
    expect(restored).toEqual(first);
  });

  test('does not coalesce sessions from different projects', async () => {
    const calls: CreativeStudioAgentSessionPersistenceRequest[] = [];
    const port: CreativeStudioAgentSessionPersistencePort = {
      async resolveOrCreateExclusive(request) {
        calls.push(request);
        return binding(request, request.projectId === 'project-a' ? conversationA : conversationB);
      },
    };
    const controller = new CreativeStudioAgentSessionController(port);

    const [first, second] = await Promise.all([
      controller.resolve(input()),
      controller.resolve(input({ projectId: 'project-b' })),
    ]);

    expect(calls.map((request) => request.projectId).sort()).toEqual(['project-a', 'project-b']);
    expect(first.conversationId).toBe(conversationA);
    expect(second.conversationId).toBe(conversationB);
  });

  test('rejects a cross-project binding returned by the persistence port', async () => {
    const port: CreativeStudioAgentSessionPersistencePort = {
      async resolveOrCreateExclusive(request) {
        return { ...binding(request), projectId: 'project-b' };
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
        return binding(request);
      },
    };
    const controller = new CreativeStudioAgentSessionController(port);

    const firstFailure = await controller.resolve(input()).catch((error: unknown) => error);
    expect(firstFailure instanceof Error).toBe(true);
    expect((firstFailure as Error).message).toBe('temporary backend failure');
    expect((await controller.resolve(input())).conversationId).toBe(conversationA);
    expect(attempts).toBe(2);
  });

  test('fails when the caller history key is stale before persistence is contacted', async () => {
    let called = false;
    const port: CreativeStudioAgentSessionPersistencePort = {
      async resolveOrCreateExclusive(request) {
        called = true;
        return binding(request);
      },
    };

    const failure = await new CreativeStudioAgentSessionController(port)
      .resolve(input({ historyKey: 'stale' }))
      .catch((error: unknown) => error);
    expect(failure instanceof CreativeStudioAgentSessionResolutionError).toBe(true);
    expect((failure as CreativeStudioAgentSessionResolutionError).code).toBe(
      'HISTORY_PROJECTION_MISMATCH'
    );
    expect(called).toBe(false);
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
