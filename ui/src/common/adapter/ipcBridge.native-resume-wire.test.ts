import { expect, test } from 'bun:test';
import { agentPlatform } from './ipcBridge';
import { parseConversationId } from '@/common/types/ids';
import { asDigestHex } from '@/common/types/agentPlatform/contracts';

test('native continuation keeps queued acceptance distinct from authorization and posts the exact original pause', async () => {
  const session = parseConversationId('0190f5fe-7c00-7a00-8000-000000000225');
  const request = { operation_id: 'original-operation', idempotency_key: 'explicit-resume',
    expected_pause_revision: 2, expected_checkpoint_revision: 4, expected_checkpoint_digest: asDigestHex('a'.repeat(64)), budget: {} };
  const requests: Array<{ url: string; body: unknown }> = [];
  const originalFetch = globalThis.fetch;
  try {
    globalThis.fetch = (async (url, init) => {
      requests.push({ url: String(url), body: JSON.parse(String(init?.body)) });
      return new Response(JSON.stringify({ success: true, data: { status: 'queued', operation_id: request.operation_id,
        request_event_id: 'original-resume-request', seq: 7, duplicate: false } }), {
        status: 200, headers: { 'Content-Type': 'application/json' },
      });
    }) as typeof fetch;
    const receipt = await agentPlatform.sessions.resumeExecution.invoke({ agent_session_id: session, request });
    expect(receipt.status).toBe('queued');
    expect('authorization_event_id' in receipt).toBe(false);
    expect(requests).toHaveLength(1);
    expect(requests[0].url.endsWith(`/api/agent-sessions/${session}/execution/resume`)).toBe(true);
    expect(requests[0].body).toEqual(request);
  } finally {
    globalThis.fetch = originalFetch;
  }
});
