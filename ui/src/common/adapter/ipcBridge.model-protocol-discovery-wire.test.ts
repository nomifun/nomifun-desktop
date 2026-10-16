import { afterEach, expect, test } from 'bun:test';
import { parseProviderId } from '@/common/types/ids';
import { mode } from './ipcBridge';

const originalFetch = globalThis.fetch;
afterEach(() => { globalThis.fetch = originalFetch; });
test('protocol discovery sends complete task configuration and credentials in request bodies with caller cancellation', async () => {
  const requests: Array<{ url: string; body: unknown; signal?: AbortSignal | null }> = [];
  globalThis.fetch = (async (input: string | URL | Request, init?: RequestInit) => {
    requests.push({ url: String(input), body: JSON.parse(String(init?.body)), signal: init?.signal });
    return new Response(JSON.stringify({ success: true, data: { candidates: [], elapsed_ms: 1 } }), { headers: { 'Content-Type': 'application/json' } });
  }) as typeof fetch;
  const capability = { task: 'chat' as const, protocol: 'openai.responses', connection_role: 'default', endpoint: '/responses',
    output_limit: 32_000, provider_params: { reasoning_effort: 'high', temperature: 0.2 } };
  const model = 'vendor/actual-model';
  const anonymous = { platform: 'custom', base_url: 'https://vendor.example/v1', auth_scheme: 'bearer',
    credentials: { api_keys: ['test-secret'] }, model, capability, preferred_protocol: 'openai.chat_text', apply_recommended_defaults: false };
  const controller = new AbortController();
  await mode.discoverModelProtocols.invoke(anonymous, controller.signal);
  const provider_id = parseProviderId('0190f5fe-7c00-7a00-8000-000000000211');
  await mode.discoverProviderModelProtocols.invoke({ provider_id, model, capability, preferred_protocol: 'openai.chat_text', apply_recommended_defaults: false });
  expect(new URL(requests[0].url, 'http://127.0.0.1').pathname).toBe('/api/providers/discover-model-protocols');
  expect(requests[0].body).toEqual(anonymous);
  expect(requests[0].url).not.toContain('test-secret');
  expect(requests[0].signal).toBeDefined();
  expect(new URL(requests[1].url, 'http://127.0.0.1').pathname).toBe(`/api/providers/${provider_id}/discover-model-protocols`);
  expect(requests[1].body).toEqual({ model, capability, preferred_protocol: 'openai.chat_text', apply_recommended_defaults: false });
});

test('cancelling a setup check aborts fetch without reporting a provider incompatibility', async () => {
  let received: AbortSignal | null | undefined;
  globalThis.fetch = (async (_input: string | URL | Request, init?: RequestInit) => {
    received = init?.signal;
    return new Promise((_resolve, reject) => received?.addEventListener('abort', () => reject(new DOMException('aborted', 'AbortError'))));
  }) as typeof fetch;
  const controller = new AbortController();
  const pending = mode.discoverModelProtocols.invoke({ platform: 'custom', base_url: 'https://vendor.example/v1', auth_scheme: 'none', credentials: {}, model: 'test',
    capability: { task: 'image_generation', protocol: 'openai.images', connection_role: 'default', provider_params: {} } }, controller.signal);
  controller.abort();
  await expect(pending).rejects.toMatchObject({ name: 'AbortError' });
  expect(received?.aborted).toBe(true);
});
