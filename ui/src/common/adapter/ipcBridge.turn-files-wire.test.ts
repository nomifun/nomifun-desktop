import { afterEach, expect, test } from 'bun:test';
import { parseConversationId, parseMessageId } from '@/common/types/ids';
import { conversation } from './ipcBridge';

const session = parseConversationId('0190f5fe-7c00-7a00-8000-000000000301');
const turn = parseMessageId('0190f5fe-7c00-7a00-8000-000000000302');
const turn2 = parseMessageId('0190f5fe-7c00-7a00-8000-000000000303');
const realFetch = globalThis.fetch;
afterEach(() => { globalThis.fetch = realFetch; });

test('requests all specified root Turns from the Session file projection independently of message history', async () => {
  let request: { url: string; method: string } | undefined;
  const data = [{ turn_id: turn, complete: false, files: [{ identity: '/work/a.docx', relative_path: 'a.docx', file_name: 'a.docx', operation: 'delete',
    published: { artifact_id: 'a'.repeat(64), path: '/published/file', mime_type: 'application/octet-stream', size_bytes: 22, sha256: 'a'.repeat(64) },
  }] }];
  globalThis.fetch = (async (input: string | URL | Request, init?: RequestInit) => {
    request = { url: String(input), method: init?.method ?? 'GET' };
    return new Response(JSON.stringify({ success: true, data }), { status: 200, headers: { 'Content-Type': 'application/json' } });
  }) as typeof fetch;
  const result = await conversation.turnFiles.invoke({ conversation_id: session, turn_ids: [turn, turn2] });
  const url = new URL(request!.url, 'http://127.0.0.1');
  expect(request?.method).toBe('GET');
  expect(url.pathname).toBe(`/api/agent-sessions/${session}/turn-files`);
  expect(url.searchParams.get('turn_ids')).toBe(`${turn},${turn2}`);
  expect(result).toEqual(data);
});
