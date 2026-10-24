import { afterEach, beforeEach, expect, spyOn, test } from 'bun:test';
import { parseConversationId } from '@/common/types/ids';
import { conversation } from './ipcBridge';

const realFetch = globalThis.fetch;
const params = { conversation_id: parseConversationId('0190f5fe-7c00-7a00-8000-000000000211'),
  workspace: '/workspace', path: '/workspace', search: 'index' };
let restoreErrors = () => {};
beforeEach(() => {
  const errors = spyOn(console, 'error').mockImplementation(() => {});
  restoreErrors = () => errors.mockRestore();
});
afterEach(() => { globalThis.fetch = realFetch; restoreErrors(); });

const busy = () => rejected(409, 'WORKSPACE_LIFECYCLE_BUSY');
function rejected(status: number, code: string) {
  return new Response(JSON.stringify({ code, error: 'fixture refusal' }), { status, headers: { 'Content-Type': 'application/json' } });
}
function entries() {
  return new Response(JSON.stringify({ success: true, data: [{ name: 'index.html', type: 'file' }] }),
    { headers: { 'Content-Type': 'application/json' } });
}

test('the workspace bridge retries lifecycle contention and publishes the successful listing with the original query', async () => {
  const requests: string[] = [];
  globalThis.fetch = (async (url, init) => {
    requests.push(String(url));
    expect(init?.method).toBe('GET');
    return requests.length === 1 ? busy() : entries();
  }) as typeof fetch;
  const result = await conversation.getWorkspace.invoke(params);
  expect(requests).toHaveLength(2);
  expect(requests[0]).toBe(requests[1]);
  expect(requests[0]).toContain('search=index');
  expect(result[0].children?.[0]).toMatchObject({ name: 'index.html', fullPath: '/workspace/index.html' });
});

test('workspace lifecycle retries stop after the bounded budget and retain the precise refusal', async () => {
  let calls = 0;
  globalThis.fetch = (async () => { calls++; return busy(); }) as typeof fetch;
  await expect(conversation.getWorkspace.invoke(params)).rejects.toMatchObject({ status: 409, code: 'WORKSPACE_LIFECYCLE_BUSY' });
  expect(calls).toBe(4);
});

test('other conflicts, missing paths, permissions and IO failures are never retried as lifecycle contention', async () => {
  for (const [status, code] of [[409, 'CONFLICT'], [409, 'APP_DEVELOPMENT_BUSY'], [404, 'NOT_FOUND'],
    [403, 'FORBIDDEN'], [500, 'INTERNAL_ERROR'], [500, 'WORKSPACE_LIFECYCLE_BUSY']] as const) {
    let calls = 0;
    globalThis.fetch = (async () => { calls++; return rejected(status, code); }) as typeof fetch;
    await expect(conversation.getWorkspace.invoke(params)).rejects.toMatchObject({ status, code });
    expect(calls).toBe(1);
  }
});

test('cancelling a workspace read during busy backoff prevents later HTTP attempts', async () => {
  let calls = 0;
  globalThis.fetch = (async () => { calls++; return busy(); }) as typeof fetch;
  const controller = new AbortController();
  const pending = conversation.getWorkspace.invoke(params, controller.signal);
  await new Promise((resolve) => setTimeout(resolve, 20));
  controller.abort();
  await expect(pending).rejects.toMatchObject({ name: 'AbortError' });
  await new Promise((resolve) => setTimeout(resolve, 300));
  expect(calls).toBe(1);
});

test('workspace caller cancellation reaches an in-flight transport read', async () => {
  let received: AbortSignal | null | undefined;
  globalThis.fetch = (async (_url, init) => {
    received = init?.signal;
    return new Promise((_resolve, reject) => received?.addEventListener('abort', () => reject(new DOMException('aborted', 'AbortError'))));
  }) as typeof fetch;
  const controller = new AbortController();
  const pending = conversation.getWorkspace.invoke(params, controller.signal);
  controller.abort();
  await expect(pending).rejects.toMatchObject({ name: 'AbortError' });
  expect(received?.aborted).toBe(true);
});
