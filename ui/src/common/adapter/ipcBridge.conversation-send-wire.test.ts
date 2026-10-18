import { afterEach, describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { parseConversationId } from '@/common/types/ids';
import { conversation } from './ipcBridge';

const source = readFileSync(new URL('./ipcBridge.ts', import.meta.url), 'utf8');
const realFetch = globalThis.fetch;
const session = parseConversationId('0190f5fe-7c00-7a00-8000-000000000211');
const message = '0190f5fe-7c00-7a00-8000-000000000219';
afterEach(() => { globalThis.fetch = realFetch; });

describe('ordinary message intake wire contract', () => {
  test('keeps the first-input constraint on the existing strict header without a second body field', async () => {
    globalThis.fetch = (async (url, init) => {
      expect(String(url)).toContain(`/api/agent-sessions/${session}/messages`);
      expect(new Headers(init?.headers).get('X-Nomifun-Initial-Delivery')).toBe('1');
      const body = JSON.parse(String(init?.body));
      expect(body).toEqual({ idempotency_key: 'initial-key', input: { content: 'Initial request' } });
      return new Response(JSON.stringify({ success: true, data: {
        agent_session_id: session, message_id: message, status: 'accepted', replayed: false,
      } }), { status: 200, headers: { 'Content-Type': 'application/json' } });
    }) as typeof fetch;
    await conversation.sendMessage.invoke({ conversation_id: session, input: 'Initial request',
      idempotency_key: 'initial-key', initial_only: true });
  });

  test('saves ordinary input through one endpoint without selecting execution or recovery', async () => {
    const requests: Array<Record<string, unknown>> = [];
    globalThis.fetch = (async (url, init) => {
      expect(String(url)).toContain(`/api/agent-sessions/${session}/messages`);
      requests.push(JSON.parse(String(init?.body)));
      return new Response(JSON.stringify({ success: true, data: {
        agent_session_id: session, message_id: message, cursor: { seq: 99 }, status: 'accepted', replayed: true,
      } }), { status: 202, headers: { 'Content-Type': 'application/json' } });
    }) as typeof fetch;
    const receipt = await conversation.sendMessage.invoke({ conversation_id: session,
      input: 'The dependency is ready', files: ['/work/brief.txt'], inject_skills: ['office'], idempotency_key: 'same-input-key' });
    expect(receipt).toEqual({ msg_id: message, replayed: true });
    expect(requests[0]).toEqual({ idempotency_key: 'same-input-key', input: {
      content: 'The dependency is ready', files: ['/work/brief.txt'], inject_skills: ['office'],
    } });
    expect(source.includes('resumeInput:')).toBe(false);
    expect(source.includes('normalizeConversationInputAction')).toBe(false);
  });

  test.each([
    { agent_session_id: session, message_id: message, replayed: false },
    { agent_session_id: session, message_id: message, status: 'running', replayed: false },
    { agent_session_id: 'foreign', message_id: message, status: 'accepted', replayed: false },
    { agent_session_id: session, message_id: message, status: 'accepted' },
  ])('does not acknowledge a response without exact accepted-message authority', async data => {
    globalThis.fetch = (async () => new Response(JSON.stringify({ success: true, data }),
      { status: 202, headers: { 'Content-Type': 'application/json' } })) as typeof fetch;
    expect(conversation.sendMessage.invoke({ conversation_id: session, input: 'Hello', idempotency_key: 'test-key' }))
      .rejects.toThrow('canonical accepted-message receipt');
  });

  test('rejects a missing idempotency key before making a request', async () => {
    let requests = 0;
    globalThis.fetch = (async () => { requests++; throw new Error('must not request'); }) as typeof fetch;
    await expect(conversation.sendMessage.invoke({ conversation_id: session, input: 'Hello', idempotency_key: '' })).rejects.toThrow();
    expect(requests).toBe(0);
  });

  test('keeps explicit domain Turn delivery separate from ordinary input acceptance', async () => {
    globalThis.fetch = (async (url) => {
      expect(String(url)).toContain(`/api/agent-sessions/${session}/turns`);
      return new Response(JSON.stringify({ success: true, data: { message_id: message, replayed: true, completed: true,
        result_ok: false, result_error: 'provider failed', result_error_code: 'output_truncated', result_error_retryable: true } }),
      { status: 202, headers: { 'Content-Type': 'application/json' } });
    }) as typeof fetch;
    const receipt = await conversation.sendTurn.invoke({ conversation_id: session, input: 'Domain work', idempotency_key: 'domain-key' });
    expect(receipt.completed).toBe(true);
    expect(receipt.result_error_code).toBe('output_truncated');
  });
});
