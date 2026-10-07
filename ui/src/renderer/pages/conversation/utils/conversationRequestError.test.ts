import { expect, test } from 'bun:test';
import { BackendHttpError } from '@/common/adapter/httpBridge';
import { parseConversationId, parseMessageId } from '@/common/types/ids';
import type { TMessage } from '@/common/chat/chatLib';
import { conversationRequestError, hasCanonicalRequestError } from './conversationRequestError';

test('request presentation preserves structured classification and exact path without fabricating a retry', () => {
  const error = new BackendHttpError({ method: 'POST', path: '/conversation/send', status: 409, body: {
    code: 'WORKSPACE_DIRECTORY_RUNTIME_UNAVAILABLE', error: 'Directory unavailable', details: { workspace_path: '/missing/project' },
  } });
  expect(conversationRequestError(error, 'CONVERSATION_SEND_FAILED')).toEqual({
    message: 'Directory unavailable', detail: 'Directory unavailable', code: 'WORKSPACE_DIRECTORY_RUNTIME_UNAVAILABLE',
    workspacePath: '/missing/project', retryable: false,
  });
  expect(conversationRequestError(new Error('Opaque upstream text'), 'CONVERSATION_SEND_FAILED').code).toBe('CONVERSATION_SEND_FAILED');
});

test('only the newly admitted matching request replaces its HTTP fallback with the canonical error', () => {
  const conversationId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000061');
  const turnId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000062');
  const user: TMessage = { id: 'user', type: 'text', position: 'right', conversation_id: conversationId,
    msg_id: turnId, content: { content: 'Inspect this project' } };
  const error: TMessage = { id: 'error', type: 'tips', conversation_id: conversationId, turn_id: turnId,
    content: { type: 'error', content: '', error: { message: 'failed', code: 'NOMIFUN_INTERNAL_ERROR' } } };
  const failure = { error: conversationRequestError(new Error('HTTP failed'), 'CONVERSATION_SEND_FAILED'),
    timestamp: 100, input: 'Inspect this project', previousMessageIds: new Set<string>() };
  expect(hasCanonicalRequestError(failure, [user, error])).toBe(true);
  expect(hasCanonicalRequestError({ ...failure, previousMessageIds: new Set([turnId]) }, [user, error])).toBe(false);
  expect(hasCanonicalRequestError({ ...failure, input: 'Another request' }, [user, error])).toBe(false);
  expect(hasCanonicalRequestError(failure, [error])).toBe(false);
  expect(hasCanonicalRequestError({ ...failure, input: undefined }, [user, error])).toBe(false);
});
