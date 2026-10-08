/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import {
  parseConversationId,
  parseKnowledgeBaseId,
  parseMessageId,
} from '@/common/types/ids';
import {
  composeMessage,
  joinPath,
  mergeTextMessageContent,
  mergeToolCallContent,
  normalizeAgentStreamError,
  normalizeToolCallContent,
  preferTextMessageVersion,
  transformMessage,
  transformUserCreatedEvent,
} from './chatLib';

describe('structured error diagnosis', () => {
  test('retains host-classified cause and frozen execution context', () => {
    const data = {
      message: 'The Agent stopped', code: 'NOMIFUN_TASK_INCOMPLETE',
      detail: 'completion account contains blocked work; task cannot complete',
      taskIncompleteReason: 'blocked_work', agentLabel: 'Research Agent',
      agentTemplateKey: 'assistant.general', modelName: 'used-model',
      workspacePath: '/workspace/research', retryable: false,
    };
    expect(normalizeAgentStreamError(data)).toEqual(data);
  });

  test('never guesses a reason from diagnostic prose or accepts an unknown reason', () => {
    const data = { message: 'stopped', code: 'NOMIFUN_TASK_INCOMPLETE', detail: 'completion account contains blocked work;' };
    expect(normalizeAgentStreamError(data)?.taskIncompleteReason).toBeUndefined();
    expect(normalizeAgentStreamError({ ...data, taskIncompleteReason: 'made_up' })?.taskIncompleteReason).toBeUndefined();
    expect(normalizeAgentStreamError({ ...data, code: 'UNKNOWN_UPSTREAM_ERROR', taskIncompleteReason: 'blocked_work' })?.taskIncompleteReason).toBeUndefined();
  });
});

const MESSAGE_ID = parseMessageId('019b0000-0000-7000-8000-000000000001');
const SECOND_MESSAGE_ID = parseMessageId('019b0000-0000-7000-8000-000000000002');

test('late user-message acknowledgements retain persisted camera observations and device provenance', () => {
  const conversationId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000001');
  const live = transformUserCreatedEvent({ conversation_id: conversationId, msg_id: MESSAGE_ID,
    content: 'Look here', position: 'right', status: 'finish', created_at: 1,
    interaction: { kind: 'robot', robot_id: 'robot-1', connection_id: 'socket-1', request_id: 'turn-1', input_modality: 'speech', output_mode: 'spoken' },
  }, conversationId)!;
  const saved = { ...live, content: { ...live.content, observations: [{ question: 'What is here?', answer: 'A cup', observed_at: 2,
    image: { id: 'photo-1', path: '/companion/cup.jpg', mime_type: 'image/jpeg', sha256: 'a'.repeat(64) } }] } };
  const merged = preferTextMessageVersion(live, saved);
  expect(merged.content.content).toBe('Look here');
  expect(merged.content.interaction?.robot_id).toBe('robot-1');
  expect(merged.content.observations?.[0].image.id).toBe('photo-1');
});

const baseWire = (overrides: Record<string, unknown>) =>
  ({
    msg_id: MESSAGE_ID,
    conversation_id: parseConversationId('0190f5fe-7c00-7a00-8000-000000000001'),
    ...overrides,
  }) as any;

describe('typed gateway terminal errors', () => {
  const cases = [
    ['USER_LLM_PROVIDER_BILLING_REQUIRED', 'The model gateway balance is insufficient. Top up the account to continue.'],
    ['USER_LLM_PROVIDER_BILLING_REQUIRED', 'The model gateway subscription has expired. Renew the subscription to continue.'],
    ['USER_LLM_PROVIDER_BILLING_REQUIRED', 'The selected model is not included in your gateway plan. Choose an included model or change the plan.'],
    ['USER_LLM_PROVIDER_AUTH_FAILED', 'The model gateway key has expired. Create a new key and update provider credentials.'],
    ['USER_LLM_PROVIDER_RATE_LIMITED', 'The model gateway rate limited the request. Wait and retry the same model.'],
  ];
  test.each(cases)('renders %s as a failed Turn error preserving wire correlation', (code, message) => {
    const notice = transformMessage(baseWire({ type: 'error', turn_id: SECOND_MESSAGE_ID, created_at: 1234,
      data: { code, message, ownership: 'user_llm_provider', retryable: false },
    }));
    expect(notice?.type).toBe('tips');
    if (notice?.type !== 'tips') throw new Error('expected presentation notice');
    expect(notice.msg_id).toBe(MESSAGE_ID); expect(notice.turn_id).toBe(SECOND_MESSAGE_ID);
    expect(notice.created_at).toBe(1234);
    expect(notice.content).toEqual({ type: 'error', content: message,
      error: { code, message, ownership: 'user_llm_provider', retryable: false },
    });
    expect(notice.message_id).toBeUndefined();
  });
  test('keeps cron and unknown System payloads invisible without classifying prose', () => {
    for (const data of [
      { kind: 'cron_response', message: cases[0][1] },
      { kind: 'unknown', error: { code: cases[0][0], message: cases[0][1], ownership: 'user_llm_provider' } },
      null, [], cases[0][1],
    ]) expect(transformMessage(baseWire({ type: 'system', data }))).toBeUndefined();
  });
});

test('explicit thinking deltas can reopen a completed contiguous phase', () => {
  const completed = transformMessage(baseWire({ type: 'thinking', data: { content: 'Inspect. ', status: 'done' } }));
  const resumed = transformMessage(baseWire({ type: 'thinking', data: { content: 'Verify.', status: 'thinking' } }));
  const merged = composeMessage(resumed, completed ? [completed] : []);
  expect(merged).toHaveLength(1);
  expect(merged[0].content).toMatchObject({ content: 'Inspect. Verify.', status: 'thinking' });
});

test('reasoning resumes its canonical row across narration and another completed step', () => {
  const first = transformMessage(baseWire({ type: 'thinking', data: { content: 'Inspect. ', status: 'done' } }))!;
  const narration = transformMessage(baseWire({ type: 'text', msg_id: SECOND_MESSAGE_ID, data: { content: 'Reading the source.' } }))!;
  const resumed = transformMessage(baseWire({ type: 'thinking', data: { content: 'Verify.', status: 'thinking' } }))!;
  const merged = composeMessage(resumed, [first, narration]);
  expect(merged).toHaveLength(2);
  expect(merged[0].id).toBe(first.id);
  expect(merged[0].content).toMatchObject({ content: 'Inspect. Verify.', status: 'thinking' });
  expect(merged[1]).toBe(narration);
  expect(first.content).toMatchObject({ content: 'Inspect. ', status: 'done' });
});

test('cancelled tool history cannot become successful after a late frame', () => {
  const cancelled = normalizeToolCallContent({call_id:'cancelled',name:'exec_command',status:'canceled',output:'STARTED'}, 'finish');
  expect(cancelled.status).toBe('canceled');
  const completed = {...cancelled,status:'completed' as const};
  expect(mergeToolCallContent(completed, cancelled).status).toBe('canceled');
  expect(mergeToolCallContent(cancelled, completed).status).toBe('canceled');
  expect(mergeToolCallContent(cancelled, completed).artifacts).toEqual([]);
  expect(mergeToolCallContent(cancelled, {...cancelled,status:'error'}).status).toBe('error');
});

test('canonical cancelled Turn metadata survives transport normalization', () => {
  const message = transformMessage(baseWire({type:'agent_status',data:{
    backend:'nomi',status:'error',turn_summary:true,turn_state:'cancelled',finished_at_ms:3000,
  }}));
  if(message?.type!=='agent_status')throw new Error('expected status metadata');
  expect(message.content.turn_state).toBe('cancelled');
  const ordinary = transformMessage(baseWire({type:'agent_status',data:{
    backend:'nomi',status:'error',turn_state:'cancelled',
  }}));
  if(ordinary?.type!=='agent_status')throw new Error('expected agent status');
  expect(ordinary.content.turn_state).toBeUndefined();
});

describe('joinPath compatibility export', () => {
  test('preserves UNC and URI prefixes', () => {
    expect(joinPath('//server/share/project', '../cat.png')).toBe('//server/share/cat.png');
    expect(joinPath('https://example.com/assets', 'cat.png')).toBe('https://example.com/assets/cat.png');
  });
});

describe('knowledge writeback attempt ordering', () => {
  test('does not let a delayed older attempt overwrite a newer manual retry', () => {
    const merged = mergeTextMessageContent(
      {
        content: 'Final answer.',
        knowledge_writeback: {
          status: 'writing',
          attempt_id: 'attempt-2',
          started_at: 200,
          updated_at: 220,
        },
      },
      {
        content: '',
        knowledge_writeback: {
          status: 'failed',
          attempt_id: 'attempt-1',
          started_at: 100,
          // Deliberately later than attempt-2's latest progress timestamp:
          // generation order must come from started_at, not delivery time.
          updated_at: 999,
          retryable: true,
        },
      }
    );

    expect(merged.knowledge_writeback?.attempt_id).toBe('attempt-2');
    expect(merged.knowledge_writeback?.status).toBe('writing');
  });

  test('keeps terminal state monotonic within one attempt', () => {
    const merged = mergeTextMessageContent(
      {
        content: 'Final answer.',
        knowledge_writeback: {
          status: 'written',
          attempt_id: 'attempt-1',
          started_at: 100,
          updated_at: 200,
        },
      },
      {
        content: '',
        knowledge_writeback: {
          status: 'writing',
          attempt_id: 'attempt-1',
          started_at: 100,
          updated_at: 300,
        },
      }
    );

    expect(merged.knowledge_writeback?.status).toBe('written');
  });
});

describe('transformMessage runtime field normalization', () => {
  test('retains only a valid truncated-turn recovery capability on live tips', () => {
    const recovered = transformMessage(
      baseWire({
        type: 'tips',
        data: {
          content: 'cut off',
          type: 'error',
          error: { message: 'cut off', code: 'OUTPUT_TRUNCATED', retryable: true },
          recovery: {
            kind: 'continue_truncated',
            source_message_id: SECOND_MESSAGE_ID,
            failure_code: 'output_truncated',
          },
        },
      })
    );
    expect(recovered?.type).toBe('tips');
    if (recovered?.type !== 'tips') throw new Error('expected tips');
    expect(recovered.content.recovery).toEqual({
      kind: 'continue_truncated',
      source_message_id: SECOND_MESSAGE_ID,
      failure_code: 'output_truncated',
    });

    const malformed = transformMessage(
      baseWire({
        type: 'tips',
        data: {
          content: 'cut off',
          type: 'error',
          recovery: {
            kind: 'continue_truncated',
            source_message_id: 'not-a-message-id',
            failure_code: 'output_truncated',
          },
        },
      })
    );
    if (malformed?.type !== 'tips') throw new Error('expected tips');
    expect(malformed.content.recovery).toBeUndefined();
  });

  test('generic tool failure is absorbing across a late completed artifact frame', () => {
    const failed = transformMessage(
      baseWire({
        type: 'tool_call',
        data: { call_id: 'tool-1', name: 'Generate', status: 'error', output: 'failed' },
      })
    )!;
    const lateCompleted = transformMessage(
      baseWire({
        type: 'tool_call',
        data: {
          call_id: 'tool-1',
          name: 'Generate',
          status: 'completed',
          artifacts: [
            {
              id: '019b0000-0000-7000-8000-000000000002',
              kind: 'image',
              mime_type: 'image/png',
              path: '/workspace/old.png',
              relative_path: 'nomifun-artifacts/old.png',
              size_bytes: 10,
              sha256: 'a'.repeat(64),
            },
          ],
        },
      })
    )!;

    const merged = composeMessage(lateCompleted, [failed]);
    const message = merged[0];
    if (message.type !== 'tool_call') throw new Error('expected tool call');
    expect(message.content.status).toBe('error');
    expect(message.content.artifacts).toEqual([]);
  });

  test('generic tool error correction retracts an earlier completed artifact frame', () => {
    const completed = transformMessage(
      baseWire({
        type: 'tool_call',
        data: {
          call_id: 'tool-corrected',
          name: 'Generate',
          status: 'completed',
          artifacts: [
            {
              id: '019b0000-0000-7000-8000-000000000002',
              kind: 'image',
              mime_type: 'image/png',
              path: '/workspace/old.png',
              relative_path: 'nomifun-artifacts/old.png',
              size_bytes: 10,
              sha256: 'a'.repeat(64),
            },
          ],
        },
      })
    )!;
    const correction = transformMessage(
      baseWire({
        type: 'tool_call',
        data: {
          call_id: 'tool-corrected',
          name: 'Generate',
          status: 'error',
          output: 'enclosing turn failed',
          artifacts: [],
        },
      })
    )!;

    const merged = composeMessage(correction, [completed]);
    const message = merged[0];
    if (message.type !== 'tool_call') throw new Error('expected tool call');
    expect(message.content.status).toBe('error');
    expect(message.content.artifacts).toEqual([]);
  });

  test('only completed tool calls retain structurally valid durable artifact receipts', () => {
    const artifact = {
      id: '019b0000-0000-7000-8000-000000000002',
      kind: 'image',
      mime_type: 'image/png',
      path: '/workspace/nomifun-artifacts/image.png',
      relative_path: 'nomifun-artifacts/image.png',
      size_bytes: 10,
      sha256: 'a'.repeat(64),
    };
    const transform = (status: 'running' | 'completed' | 'error', value: Record<string, unknown>) =>
      transformMessage(
        baseWire({
          type: 'tool_call',
          data: { call_id: `tool-${status}`, name: 'Generate', status, artifacts: [value] },
        })
      );

    const completed = transform('completed', artifact);
    if (completed?.type !== 'tool_call') throw new Error('expected completed tool call');
    expect(completed.content.artifacts).toEqual([artifact]);

    for (const status of ['running', 'error'] as const) {
      const message = transform(status, artifact);
      if (message?.type !== 'tool_call') throw new Error('expected tool call');
      expect(message.content.artifacts).toEqual([]);
    }

    for (const malformed of [
      { ...artifact, sha256: 'not-a-sha' },
      { ...artifact, size_bytes: 0 },
      { ...artifact, path: 'relative/image.png' },
      { ...artifact, relative_path: '../old.png' },
    ]) {
      const message = transform('completed', malformed);
      if (message?.type !== 'tool_call') throw new Error('expected tool call');
      expect(message.content.artifacts).toEqual([]);
    }
  });

  test('composeMessage keeps reused tool call ids isolated by turn', () => {
    const first = transformMessage(baseWire({
      msg_id: MESSAGE_ID,
      type: 'tool_call',
      data: { call_id: 'call-1', name: 'Read', status: 'completed' },
    }))!;
    const second = transformMessage(baseWire({
      msg_id: SECOND_MESSAGE_ID,
      type: 'tool_call',
      data: { call_id: 'call-1', name: 'Read', status: 'running' },
    }))!;

    expect(composeMessage(second, [first])).toHaveLength(2);
  });

  test('serializes structured text payloads instead of leaking objects into message content', () => {
    const message = transformMessage(
      baseWire({
        type: 'text',
        data: { command: 'codex --version' },
      })
    );

    expect(message?.type).toBe('text');
    if (message?.type !== 'text') throw new Error('expected text message');
    expect(message.content.content).toBe('{\n  "command": "codex --version"\n}');
  });

  test('serializes non-string rich text content while preserving string metadata only', () => {
    const message = transformMessage(
      baseWire({
        type: 'content',
        data: {
          content: { text: 'hello' },
          sender_name: { bad: true },
          sender_backend: 'codex',
          sender_conversation_id: 'not-a-number',
        },
      })
    );

    expect(message?.type).toBe('text');
    if (message?.type !== 'text') throw new Error('expected text message');
    expect(message.content.content).toBe('{\n  "text": "hello"\n}');
    expect(message.content.senderName).toBeUndefined();
    expect(message.content.senderAgentType).toBe('codex');
    expect(message.content.senderConversationId).toBeUndefined();
  });

  test('maps external collaboration fields to the single Agent message shape', () => {
    const message = transformMessage(
      baseWire({
        type: 'content',
        data: {
          content: 'Delegated result',
          teammate_message: true,
          sender_name: 'Researcher',
          sender_backend: 'nomi',
          sender_conversation_id: '0190f5fe-7c00-7a00-8000-000000000007',
        },
      })
    );

    expect(message?.type).toBe('text');
    if (message?.type !== 'text') throw new Error('expected text message');
    expect(message.content).toMatchObject({
      content: 'Delegated result',
      agentMessage: true,
      senderName: 'Researcher',
      senderAgentType: 'nomi',
      senderConversationId: '0190f5fe-7c00-7a00-8000-000000000007',
    });
  });

  test('normalizes tips content and type from malformed payloads', () => {
    const message = transformMessage(
      baseWire({
        type: 'tips',
        data: {
          content: { message: 'rate limited' },
          type: 'unexpected',
        },
      })
    );

    expect(message?.type).toBe('tips');
    if (message?.type !== 'tips') throw new Error('expected tips message');
    expect(message.content.type).toBe('warning');
    expect(message.content.content).toBe('{\n  "message": "rate limited"\n}');
  });

  test('preserves canonical message and owning turn identities for terminal errors', () => {
    const terminalMessageId = parseMessageId('019b0000-0000-7000-8000-000000000010');
    const turnId = parseMessageId('019b0000-0000-7000-8000-000000000011');
    const message = transformMessage(
      baseWire({
        msg_id: terminalMessageId,
        turn_id: turnId,
        type: 'error',
        data: { message: 'rate limited', code: 'USER_LLM_PROVIDER_RATE_LIMITED' },
      })
    );

    expect(message?.type).toBe('tips');
    expect(message?.id).not.toBe(terminalMessageId);
    expect(message?.msg_id).toBe(terminalMessageId);
    expect(message?.turn_id).toBe(turnId);
  });

  test('preserves owning turn identity on non-terminal stream rows', () => {
    const turnId = parseMessageId('019b0000-0000-7000-8000-000000000012');
    const message = transformMessage(
      baseWire({
        turn_id: turnId,
        type: 'tool_call',
        data: { call_id: 'tool-1', name: 'Generate', status: 'running' },
      })
    );

    expect(message?.type).toBe('tool_call');
    expect(message?.turn_id).toBe(turnId);
  });

  test('normalizes thinking content, subject, status, and duration defensively', () => {
    const message = transformMessage(
      baseWire({
        type: 'thinking',
        data: {
          content: { step: 'scan' },
          subject: { title: 'Audit' },
          status: 'bad-status',
          duration_ms: '500',
        },
      })
    );

    expect(message?.type).toBe('thinking');
    if (message?.type !== 'thinking') throw new Error('expected thinking message');
    expect(message.content.content).toBe('{\n  "step": "scan"\n}');
    expect(message.content.subject).toBe('{\n  "title": "Audit"\n}');
    expect(message.content.status).toBe('thinking');
    expect(message.content.duration).toBeUndefined();
  });

  test('drops malformed tool_group content to an empty array', () => {
    const message = transformMessage(
      baseWire({
        type: 'tool_group',
        data: { call_id: 'tool-1', status: 'Executing' },
      })
    );

    expect(message?.type).toBe('tool_group');
    if (message?.type !== 'tool_group') throw new Error('expected tool_group message');
    expect(message.content).toEqual([]);
  });

  test('preserves disconnected agent status so historical rows stay hidden', () => {
    const message = transformMessage(
      baseWire({
        type: 'agent_status',
        data: {
          backend: { name: 'codex' },
          status: 'disconnected',
        },
      })
    );

    expect(message?.type).toBe('agent_status');
    if (message?.type !== 'agent_status') throw new Error('expected agent_status message');
    expect(message.content.backend).toBe('{\n  "name": "codex"\n}');
    expect(message.content.status).toBe('disconnected');
  });

  test('keeps canonical turn wall-clock timing on status metadata', () => {
    const message = transformMessage(
      baseWire({
        type: 'agent_status',
        data: {
          backend: 'nomi',
          status: 'prepared',
          turn_summary: true,
          started_at_ms: 4_000_000,
          finished_at_ms: 4_002_000,
        },
      })
    );

    expect(message?.type).toBe('agent_status');
    if (message?.type !== 'agent_status') throw new Error('expected agent_status message');
    expect(message.content.started_at_ms).toBe(4_000_000);
    expect(message.content.finished_at_ms).toBe(4_002_000);
  });

  test('preserves persisted knowledge writeback state when hydrating text messages', () => {
    const message = transformMessage(
      baseWire({
        type: 'content',
        data: {
          content: 'Final answer.',
          knowledge_writeback: {
            status: 'failed',
            attempt_id: 'attempt-1',
            retryable: true,
            failures: [{
              kb_id: parseKnowledgeBaseId('019b0000-0000-7000-8000-000000000001'),
              rel_path: 'notes.md',
              error: 'disk full',
            }],
          },
        },
      })
    );

    expect(message?.type).toBe('text');
    if (message?.type !== 'text') throw new Error('expected text message');
    expect(message.content.content).toBe('Final answer.');
    expect(message.content.knowledge_writeback?.status).toBe('failed');
    expect(message.content.knowledge_writeback?.retryable).toBe(true);
    expect(message.content.knowledge_writeback?.failures?.[0]?.error).toBe('disk full');
  });

  test('converts live user-created events into right-side messages for the active conversation', () => {
    const message = transformUserCreatedEvent(
      {
        conversation_id: parseConversationId('0190f5fe-7c00-7a00-8000-000000000002'),
        msg_id: MESSAGE_ID,
        content: 'from IM',
        position: 'right',
        status: 'finish',
        channel_platform: 'telegram',
        created_at: 1234,
        display_at_ms: 1700000000000,
      },
      parseConversationId('0190f5fe-7c00-7a00-8000-000000000002')
    );

    expect(message?.type).toBe('text');
    if (message?.type !== 'text') throw new Error('expected text message');
    expect(message.conversation_id).toBe('0190f5fe-7c00-7a00-8000-000000000002');
    expect(message.msg_id).toBe(MESSAGE_ID);
    expect(message.position).toBe('right');
    expect(message.status).toBe('finish');
    expect(message.created_at).toBe(1234);
    expect(message.content.display_at_ms).toBe(1700000000000);
    expect(message.created_at).toBe(1234);
    expect(message.content.content).toBe('from IM');
  });

  test('ignores user-created events for other conversations and hidden messages', () => {
    const baseEvent = {
      conversation_id: parseConversationId('0190f5fe-7c00-7a00-8000-000000000002'),
      msg_id: MESSAGE_ID,
      content: 'from IM',
      position: 'right' as const,
      status: 'finish',
      created_at: 1234,
    };

    expect(transformUserCreatedEvent(baseEvent, parseConversationId('0190f5fe-7c00-7a00-8000-000000000003'))).toBeUndefined();
    expect(transformUserCreatedEvent({ ...baseEvent, hidden: true }, parseConversationId('0190f5fe-7c00-7a00-8000-000000000002'))).toBeUndefined();
  });
});
