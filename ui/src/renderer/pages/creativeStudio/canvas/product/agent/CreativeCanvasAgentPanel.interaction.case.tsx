/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import assert from 'node:assert/strict';
import i18next from 'i18next';
import React, { useCallback, useState } from 'react';
import { initReactI18next } from 'react-i18next';
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from '@testing-library/react';

import {
  parseConversationId,
  parseMessageId,
  parseProviderId,
} from '@/common/types/ids';
import { BackendHttpError } from '@/common/adapter/httpBridge';
import { serializeCreativeStudioAgentHistory } from '../../../agent/adapters';
import type {
  CreativeStudioAgentChatPort,
  CreativeStudioAgentMessage,
} from '../../../agent';
import type { CreativeChatSessionReference } from '../../../domain';
import type { CreativeCanvasAgentContextSnapshot } from './context';
import CreativeCanvasAgentPanel from './CreativeCanvasAgentPanel';

await i18next.use(initReactI18next).init({
  lng: 'en-US',
  fallbackLng: 'en-US',
  resources: {
    'en-US': {
      translation: {},
    },
  },
  interpolation: { escapeValue: false },
});

const SUCCESS_MARKER = 'creative-canvas-agent-panel-interaction:ok';
const CANVAS_ID = '0190f5fe-7c00-7a00-8000-000000000901';
const SESSION_ID = '0190f5fe-7c00-7a00-8000-000000000902';
const PROVIDER_ID = parseProviderId(
  '0190f5fe-7c00-7a00-8000-000000000903'
);
const CONVERSATION_ID = parseConversationId(
  '0190f5fe-7c00-7a00-8000-000000000904'
);
const IDEMPOTENCY_KEY = '0190f5fe-7c00-7a00-8000-000000000905';
const TRANSIENT_USER_ID = '0190f5fe-7c00-7a00-8000-000000000906';
const TRANSIENT_ASSISTANT_ID =
  '0190f5fe-7c00-7a00-8000-000000000907';
const DURABLE_USER_ID = parseMessageId(
  '0190f5fe-7c00-7a00-8000-000000000908'
);
const DURABLE_ASSISTANT_ID = parseMessageId(
  '0190f5fe-7c00-7a00-8000-000000000909'
);
const RECOVERED_USER_ID = parseMessageId(
  '0190f5fe-7c00-7a00-8000-000000000910'
);
const RECOVERED_ASSISTANT_ID = parseMessageId(
  '0190f5fe-7c00-7a00-8000-000000000911'
);
const RECOVERED_PENDING_KEY =
  '0190f5fe-7c00-7a00-8000-000000000912';
const MODEL = { providerId: PROVIDER_ID, model: 'qa-creative-chat' } as const;
const PROMPT = '整理当前画布并给出下一步方案';
const TERMINAL_FAILURE_MESSAGE = '模型请求过于频繁，请稍后重试';
const BACKEND_FAILURE_MESSAGE = '模型服务暂时不可用';

const planningContext: CreativeCanvasAgentContextSnapshot = {
  kind: 'nomifun.creative-studio.canvas-context',
  version: 1,
  canvasId: CANVAS_ID,
  canvasRevision: '1',
  selectedNodeIds: [],
  nodes: [],
  connections: [],
  totalNodeCount: 0,
  totalConnectionCount: 0,
  truncated: false,
};

const baseProps = {
  canvasId: CANVAS_ID,
  planningContext,
  disabled: false,
  onApplyCanvasOps: async () => undefined,
  onCollapse: () => undefined,
  createId: (() => {
    const ids = [IDEMPOTENCY_KEY, TRANSIENT_USER_ID, TRANSIENT_ASSISTANT_ID];
    return () => {
      const id = ids.shift();
      if (!id) throw new Error('Creative Canvas Agent test exhausted UUIDs');
      return id;
    };
  })(),
  now: () => 1_000,
};

const flushReact = async (): Promise<void> => {
  await new Promise<void>((resolve) => setImmediate(resolve));
  await new Promise<void>((resolve) => setImmediate(resolve));
};

const within = async <T,>(
  operation: Promise<T>,
  label: string
): Promise<T> => {
  let timeout: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      operation,
      new Promise<never>((_, reject) => {
        timeout = setTimeout(
          () => reject(new Error(`Timed out: ${label}`)),
          3_000
        );
      }),
    ]);
  } finally {
    if (timeout !== undefined) clearTimeout(timeout);
  }
};

const verifyHydrationTransition = async (): Promise<void> => {
  const view = render(
    <CreativeCanvasAgentPanel
      {...baseProps}
      hydrated={false}
      sessions={[]}
      activeSessionId={null}
      onPersist={async () => undefined}
    />
  );

  assert.ok(screen.getByText('Loading conversation'));
  view.rerender(
    <CreativeCanvasAgentPanel
      {...baseProps}
      hydrated
      sessions={[]}
      activeSessionId={null}
      onPersist={async () => undefined}
    />
  );

  await waitFor(() => {
    assert.equal(screen.queryByText('Loading conversation'), null);
    const input = screen.getByPlaceholderText(
      'Describe a creative goal or continue the current discussion'
    ) as HTMLTextAreaElement;
    assert.equal(input.disabled, false);
  });
  cleanup();
};

const verifyLocalPendingTurnKeepsTranscript = async (): Promise<void> => {
  const initialSession: CreativeChatSessionReference = {
    id: SESSION_ID,
    title: '画布讨论',
    messageIds: [],
    model: MODEL,
    pendingTurn: null,
    createdAt: 1,
    updatedAt: 1,
  };
  let resolverCalls = 0;
  let releaseTurn!: () => void;
  let markTurnStarted!: () => void;
  const turnGate = new Promise<void>((resolve) => {
    releaseTurn = resolve;
  });
  const turnStarted = new Promise<void>((resolve) => {
    markTurnStarted = resolve;
  });
  const chatPort: CreativeStudioAgentChatPort = {
    async *runTurn(request) {
      markTurnStarted();
      await turnGate;
      yield {
        type: 'activity',
        label: '正在分析当前画布',
      };
      yield {
        type: 'assistant-delta',
        delta: '已整理',
      };
      const history: readonly CreativeStudioAgentMessage[] = [
        ...request.history,
        {
          id: DURABLE_USER_ID,
          role: 'user',
          status: 'complete',
          text: request.prompt,
        },
        {
          id: DURABLE_ASSISTANT_ID,
          role: 'assistant',
          status: 'complete',
          text: '已整理当前画布。',
        },
      ];
      yield { type: 'history-reconciled', history };
      yield { type: 'completed', assistantMessageId: DURABLE_ASSISTANT_ID };
    },
  };

  const ControlledPanel: React.FC = () => {
    const [document, setDocument] = useState<{
      sessions: readonly CreativeChatSessionReference[];
      activeSessionId: string | null;
    }>({
      sessions: [initialSession],
      activeSessionId: initialSession.id,
    });
    const resolveSession = useCallback(
      async (
        input: Parameters<
          NonNullable<
            React.ComponentProps<typeof CreativeCanvasAgentPanel>['resolveSession']
          >
        >[0]
      ) => {
        resolverCalls += 1;
        const history: readonly CreativeStudioAgentMessage[] = [];
        return {
          binding: {
            ownership: 'creative-studio-exclusive' as const,
            canvasId: input.canvasId,
            sessionId: input.sessionId,
            conversationId: CONVERSATION_ID,
            model: input.model,
            historyKey: serializeCreativeStudioAgentHistory(history),
          },
          history,
          appliedProposalMessageIds: [],
          created: false,
        };
      },
      []
    );
    const persistDocument = useCallback(
      async (
        sessions: readonly CreativeChatSessionReference[],
        activeSessionId: string | null
      ) => {
        setDocument({
          sessions: structuredClone([...sessions]),
          activeSessionId,
        });
      },
      []
    );
    return (
      <CreativeCanvasAgentPanel
        {...baseProps}
        hydrated
        sessions={document.sessions}
        activeSessionId={document.activeSessionId}
        resolveSession={resolveSession}
        chatPort={chatPort}
        onPersist={persistDocument}
      />
    );
  };

  render(<ControlledPanel />);
  await act(flushReact);
  assert.equal(screen.queryByText('Loading conversation'), null);
  assert.equal(resolverCalls, 1);

  fireEvent.change(
    screen.getByPlaceholderText(
      'Describe a creative goal or continue the current discussion'
    ),
    { target: { value: PROMPT } }
  );
  fireEvent.click(screen.getByRole('button', { name: 'Send to Agent' }));
  await act(async () => {
    await within(turnStarted, 'Creative Canvas Agent turn start');
  });

  // Give the controlled Canvas document update and its load effect enough time
  // to run. The locally admitted pending turn must keep owning the transcript.
  await act(flushReact);
  await waitFor(() => {
    assert.ok(screen.getByText(PROMPT));
    assert.equal(screen.queryByText('Loading conversation'), null);
  });
  assert.equal(
    resolverCalls,
    1,
    'the pending-turn prop echo must not launch a competing authority reload'
  );

  await act(async () => {
    releaseTurn();
    await flushReact();
  });
  await waitFor(() => {
    assert.ok(screen.getByText('已整理当前画布。'));
  });
  assert.equal(resolverCalls, 1);
  cleanup();
};

const verifyTerminalFailureRendersOnce = async (): Promise<void> => {
  const initialSession: CreativeChatSessionReference = {
    id: SESSION_ID,
    title: '失败展示会话',
    messageIds: [],
    model: MODEL,
    pendingTurn: null,
    createdAt: 1,
    updatedAt: 1,
  };
  const ids = [IDEMPOTENCY_KEY, TRANSIENT_USER_ID, TRANSIENT_ASSISTANT_ID];
  const createId = () => {
    const id = ids.shift();
    if (!id) throw new Error('Terminal failure test exhausted UUIDs');
    return id;
  };
  const chatPort: CreativeStudioAgentChatPort = {
    async *runTurn() {
      yield {
        type: 'failed',
        code: 'USER_LLM_PROVIDER_RATE_LIMITED',
        message: TERMINAL_FAILURE_MESSAGE,
        retryable: true,
      };
    },
  };

  const ControlledFailurePanel: React.FC = () => {
    const [document, setDocument] = useState<{
      sessions: readonly CreativeChatSessionReference[];
      activeSessionId: string | null;
    }>({
      sessions: [initialSession],
      activeSessionId: initialSession.id,
    });
    const resolveSession = useCallback(
      async (
        input: Parameters<
          NonNullable<
            React.ComponentProps<typeof CreativeCanvasAgentPanel>['resolveSession']
          >
        >[0]
      ) => {
        const history: readonly CreativeStudioAgentMessage[] = [];
        return {
          binding: {
            ownership: 'creative-studio-exclusive' as const,
            canvasId: input.canvasId,
            sessionId: input.sessionId,
            conversationId: CONVERSATION_ID,
            model: input.model,
            historyKey: serializeCreativeStudioAgentHistory(history),
          },
          history,
          appliedProposalMessageIds: [],
          created: false,
        };
      },
      []
    );
    const persistDocument = useCallback(
      async (
        sessions: readonly CreativeChatSessionReference[],
        activeSessionId: string | null
      ) => {
        setDocument({
          sessions: structuredClone([...sessions]),
          activeSessionId,
        });
      },
      []
    );
    return (
      <CreativeCanvasAgentPanel
        {...baseProps}
        createId={createId}
        hydrated
        sessions={document.sessions}
        activeSessionId={document.activeSessionId}
        resolveSession={resolveSession}
        chatPort={chatPort}
        onPersist={persistDocument}
      />
    );
  };

  const view = render(<ControlledFailurePanel />);
  await act(flushReact);
  fireEvent.change(
    screen.getByPlaceholderText(/creative goal|创作目标/i),
    { target: { value: PROMPT } }
  );
  fireEvent.click(screen.getByRole('button', { name: /send to agent|发送给 agent/i }));

  await waitFor(() => {
    assert.equal(screen.getAllByText(TERMINAL_FAILURE_MESSAGE).length, 1);
    assert.equal(
      view.container.querySelectorAll('[data-agent-panel-error]').length,
      0
    );
  });
  const rendered = view.container.textContent ?? '';
  assert.equal(rendered.includes('USER_LLM_PROVIDER_RATE_LIMITED'), false);
  assert.equal(rendered.includes('"message"'), false);
  cleanup();
};

const verifyBackendHttpErrorUsesBackendMessage = async (): Promise<void> => {
  const initialSession: CreativeChatSessionReference = {
    id: SESSION_ID,
    title: '后端错误会话',
    messageIds: [],
    model: MODEL,
    pendingTurn: null,
    createdAt: 1,
    updatedAt: 1,
  };
  const backendError = new BackendHttpError({
    method: 'POST',
    path: '/api/creative-studio/canvas-agent-sessions/resolve',
    status: 503,
    body: {
      success: false,
      error: BACKEND_FAILURE_MESSAGE,
      code: 'PROVIDER_UNAVAILABLE',
      details: { diagnostic: 'RAW_BACKEND_DETAIL' },
    },
  });
  const view = render(
    <CreativeCanvasAgentPanel
      {...baseProps}
      hydrated
      sessions={[initialSession]}
      activeSessionId={initialSession.id}
      resolveSession={async () => {
        throw backendError;
      }}
      onPersist={async () => undefined}
    />
  );

  await waitFor(() => {
    assert.equal(screen.getAllByText(BACKEND_FAILURE_MESSAGE).length, 1);
  });
  const rendered = view.container.textContent ?? '';
  assert.equal(rendered.includes('Backend POST'), false);
  assert.equal(rendered.includes('PROVIDER_UNAVAILABLE'), false);
  assert.equal(rendered.includes('RAW_BACKEND_DETAIL'), false);
  assert.equal(rendered.includes('{"success"'), false);
  cleanup();
};

const verifyCompletedPendingTurnRecovery = async (): Promise<void> => {
  const recoveredHistory: readonly CreativeStudioAgentMessage[] = [
    {
      id: RECOVERED_USER_ID,
      role: 'user',
      status: 'complete',
      text: '恢复上一轮',
    },
    {
      id: RECOVERED_ASSISTANT_ID,
      role: 'assistant',
      status: 'complete',
      text: '上一轮已由后台完成。',
    },
  ];
  const pendingSession: CreativeChatSessionReference = {
    id: SESSION_ID,
    title: '待恢复会话',
    messageIds: [],
    model: MODEL,
    pendingTurn: {
      idempotencyKey: RECOVERED_PENDING_KEY,
      prompt: '恢复上一轮',
      modelInput: '恢复上一轮',
      skillIds: ['creative-studio-canvas'],
      createdAt: 2,
    },
    createdAt: 1,
    updatedAt: 2,
  };
  let resolverCalls = 0;
  let persistedSession: CreativeChatSessionReference | undefined;

  const ControlledRecoveryPanel: React.FC = () => {
    const [document, setDocument] = useState<{
      sessions: readonly CreativeChatSessionReference[];
      activeSessionId: string | null;
    }>({
      sessions: [pendingSession],
      activeSessionId: pendingSession.id,
    });
    const resolveSession = useCallback(
      async (
        input: Parameters<
          NonNullable<
            React.ComponentProps<typeof CreativeCanvasAgentPanel>['resolveSession']
          >
        >[0]
      ) => {
        resolverCalls += 1;
        return {
          binding: {
            ownership: 'creative-studio-exclusive' as const,
            canvasId: input.canvasId,
            sessionId: input.sessionId,
            conversationId: CONVERSATION_ID,
            model: input.model,
            historyKey: serializeCreativeStudioAgentHistory(recoveredHistory),
          },
          history: recoveredHistory,
          appliedProposalMessageIds: [],
          created: false,
        };
      },
      []
    );
    const persistDocument = useCallback(
      async (
        sessions: readonly CreativeChatSessionReference[],
        activeSessionId: string | null
      ) => {
        persistedSession = sessions.find(
          (session) => session.id === activeSessionId
        );
        setDocument({
          sessions: structuredClone([...sessions]),
          activeSessionId,
        });
      },
      []
    );
    return (
      <CreativeCanvasAgentPanel
        {...baseProps}
        hydrated
        sessions={document.sessions}
        activeSessionId={document.activeSessionId}
        resolveSession={resolveSession}
        chatPort={{
          async *runTurn() {
            throw new Error('A completed pending turn must not be submitted again');
          },
        }}
        onPersist={persistDocument}
      />
    );
  };

  render(<ControlledRecoveryPanel />);
  await waitFor(() => {
    assert.ok(screen.getByText('上一轮已由后台完成。'));
  });
  assert.equal(resolverCalls, 1);
  assert.deepEqual(persistedSession?.messageIds, [
    RECOVERED_USER_ID,
    RECOVERED_ASSISTANT_ID,
  ]);
  assert.equal(persistedSession?.pendingTurn, null);
  cleanup();
};

const verifyCompletedTurnRecoveryAfterLegacyFenceLoss = async (): Promise<void> => {
  const recoveredHistory: readonly CreativeStudioAgentMessage[] = [
    {
      id: RECOVERED_USER_ID,
      role: 'user',
      status: 'complete',
      text: '恢复丢失引用',
    },
    {
      id: RECOVERED_ASSISTANT_ID,
      role: 'assistant',
      status: 'complete',
      text: '已恢复旧版本遗漏的会话。',
    },
  ];
  const unfencedSession: CreativeChatSessionReference = {
    id: SESSION_ID,
    title: '旧版本会话',
    messageIds: [],
    model: MODEL,
    pendingTurn: null,
    createdAt: 1,
    updatedAt: 2,
  };
  let persistedSession: CreativeChatSessionReference | undefined;

  const ControlledRecoveryPanel: React.FC = () => {
    const [document, setDocument] = useState<{
      sessions: readonly CreativeChatSessionReference[];
      activeSessionId: string | null;
    }>({
      sessions: [unfencedSession],
      activeSessionId: unfencedSession.id,
    });
    const resolveSession = useCallback(
      async (
        input: Parameters<
          NonNullable<
            React.ComponentProps<typeof CreativeCanvasAgentPanel>['resolveSession']
          >
        >[0]
      ) => ({
        binding: {
          ownership: 'creative-studio-exclusive' as const,
          canvasId: input.canvasId,
          sessionId: input.sessionId,
          conversationId: CONVERSATION_ID,
          model: input.model,
          historyKey: serializeCreativeStudioAgentHistory(recoveredHistory),
        },
        history: recoveredHistory,
        appliedProposalMessageIds: [],
        created: false,
      }),
      []
    );
    const persistDocument = useCallback(
      async (
        sessions: readonly CreativeChatSessionReference[],
        activeSessionId: string | null
      ) => {
        persistedSession = sessions.find(
          (session) => session.id === activeSessionId
        );
        setDocument({
          sessions: structuredClone([...sessions]),
          activeSessionId,
        });
      },
      []
    );
    return (
      <CreativeCanvasAgentPanel
        {...baseProps}
        hydrated
        sessions={document.sessions}
        activeSessionId={document.activeSessionId}
        resolveSession={resolveSession}
        chatPort={{
          async *runTurn() {
            throw new Error('Recovered durable history must not be submitted again');
          },
        }}
        onPersist={persistDocument}
      />
    );
  };

  render(<ControlledRecoveryPanel />);
  await waitFor(() => {
    assert.ok(screen.getByText('已恢复旧版本遗漏的会话。'));
  });
  assert.deepEqual(persistedSession?.messageIds, [
    RECOVERED_USER_ID,
    RECOVERED_ASSISTANT_ID,
  ]);
  assert.equal(persistedSession?.pendingTurn, null);
  cleanup();
};

const run = async (): Promise<void> => {
  const originalConsoleError = console.error;
  console.error = (...args: unknown[]) => {
    if (
      args.length === 2 &&
      args[0] === '`NaN` is an invalid value for the `%s` css style property.' &&
      args[1] === 'height'
    ) {
      return;
    }
    originalConsoleError(...args);
  };
  try {
    await verifyHydrationTransition();
    await verifyLocalPendingTurnKeepsTranscript();
    await verifyTerminalFailureRendersOnce();
    await verifyBackendHttpErrorUsesBackendMessage();
    await verifyCompletedPendingTurnRecovery();
    await verifyCompletedTurnRecoveryAfterLegacyFenceLoss();
    await flushReact();
  } finally {
    console.error = originalConsoleError;
  }
};

try {
  await run();
  console.log(SUCCESS_MARKER);
  await new Promise<void>((resolve) => setImmediate(resolve));
  process.exit(0);
} catch (error) {
  console.error(error);
  await new Promise<void>((resolve) => setImmediate(resolve));
  process.exit(1);
}
