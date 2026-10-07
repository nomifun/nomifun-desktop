import { describe, expect, test } from 'bun:test';
import { parseConversationId, parseMessageId } from '@/common/types/ids';
import { conversationPauseError, conversationPauseErrorMessage } from './conversationPauseError';
import type { ConversationPauseNotice } from './conversationRuntime';

const conversationId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000981');
const turnId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000982');
const pause: ConversationPauseNotice = {
  turnId, reason: 'EXECUTION_MODEL_PROVIDER_UNAVAILABLE', cleanupProven: true, pausedAt: 123,
  error: { message: '', agentLabel: 'Admitted Agent', modelName: 'admitted-model',
    workspacePath: '/fixture', agentTemplateKey: 'assistant.general' },
};

describe('canonical pause error presentation', () => {
  test.each([
    ['EXECUTION_MODEL_PROVIDER_UNAVAILABLE', 'EXECUTION_MODEL_PROVIDER_UNAVAILABLE'],
    ['EXECUTION_MODEL_AUTHENTICATION_FAILED', 'USER_LLM_PROVIDER_AUTH_FAILED'],
    ['EXECUTION_MODEL_RATE_LIMITED', 'USER_LLM_PROVIDER_RATE_LIMITED'],
    ['EXECUTION_MODEL_PROMPT_TOO_LONG', 'USER_LLM_PROVIDER_CONTEXT_TOO_LARGE'],
    ['EXECUTION_MODEL_INVALID_REQUEST', 'USER_LLM_PROVIDER_INVALID_REQUEST'],
    ['EXECUTION_MODEL_CREDENTIAL_REFERENCE_MISSING', 'USER_LLM_PROVIDER_CONFIG_ERROR'],
    ['EXECUTION_MODEL_ROUTE_REVISION_MISMATCH', 'NOMIFUN_SESSION_CONFIGURATION_CHANGED'],
    ['EXECUTION_MODEL_STREAM_INTERRUPTED', 'EXECUTION_MODEL_STREAM_INTERRUPTED'],
    ['EXECUTION_SESSION_PAYLOAD_BUDGET', 'EXECUTION_SESSION_PAYLOAD_BUDGET'],
  ])('maps only the exact cause %s', (reason, code) => {
    const error = conversationPauseError({ ...pause, reason });
    expect(error.code).toBe(code);
    expect(error.detail).toBe(reason);
    expect(error.retryable).toBe(false);
  });

  test('unconfirmed cleanup overrides the cause without granting recovery authority', () => {
    const error = conversationPauseError({ ...pause, cleanupProven: false });
    expect(error.code).toBe('EXECUTION_CLEANUP_UNCONFIRMED');
    expect(error.detail).toBe('EXECUTION_MODEL_PROVIDER_UNAVAILABLE');
    expect(error.retryable).toBe(false);
    expect(error.resolution).toBeUndefined();
  });

  test('missing or unknown causes cannot expose private prose or claim a precise failure', () => {
    for (const reason of [undefined, 'private upstream diagnostic', 'FIXTURE_PROVIDER_SECRET',
      'EXECUTION_MODEL_AUTHENTICATION_FAILED with extra detail']) {
      const error = conversationPauseError({ ...pause, reason, error: {
        ...pause.error!, message: 'private message', detail: 'private detail', resolution: { kind: 'retry' },
      } });
      expect(error.code).toBe('EXECUTION_PAUSED');
      expect(error.message).toBe('');
      expect(error.detail).toBeUndefined();
      expect(error.resolution).toBeUndefined();
    }
  });

  test('the shared message is display-only and keeps exact metadata without inventing a terminal', () => {
    const message = conversationPauseErrorMessage(conversationId, pause);
    expect(message.id).toBe(`pause-error:${turnId}:123`);
    expect(message.conversation_id).toBe(conversationId);
    expect(message.turn_id).toBe(turnId);
    expect(message.created_at).toBe(123);
    expect(message.msg_id).toBeUndefined();
    expect(message.message_id).toBeUndefined();
    expect(message.status).toBeUndefined();
    expect(message.content.type).toBe('error');
    expect(message.content.finished_at_ms).toBeUndefined();
    expect(message.content.error).toMatchObject(pause.error!);
    expect(message.content.execution_pause).toEqual({ reason: pause.reason, cleanupProven: true });
    expect(conversationPauseErrorMessage(conversationId, { ...pause, pausedAt: 456 }).id).not.toBe(message.id);
  });
});
