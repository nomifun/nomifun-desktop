import { afterEach, expect, test } from 'bun:test';
import { parseProviderId } from '@/common/types/ids';
import { mode } from './ipcBridge';

const originalFetch = globalThis.fetch;
afterEach(() => { globalThis.fetch = originalFetch; });
test('actual-model protocol detection sends complete draft capability and credentials in the body for anonymous and stored providers', async () => {
  const requests: Array<{ url: string; body: unknown }> = [];
  globalThis.fetch = (async (input: string | URL | Request, init?: RequestInit) => {
    requests.push({ url: String(input), body: JSON.parse(String(init?.body)) });
    return new Response(JSON.stringify({ success: true, data: { attempts: [], elapsed_ms: 1 } }), { headers: { 'Content-Type': 'application/json' } });
  }) as typeof fetch;
  const capability = { task: 'chat' as const, protocol: 'openai.responses', connection_role: 'default', endpoint: '/responses',
    output_limit: 32_000, provider_params: { reasoning_effort: 'high', temperature: 0.2 } };
  const model = 'vendor/actual-model';
  const anonymous = { platform: 'custom', base_url: 'https://vendor.example/v1', auth_scheme: 'bearer',
    credentials: { api_keys: ['test-secret'] }, model, capability };
  await mode.detectChatProtocol.invoke(anonymous);
  const provider_id = parseProviderId('0190f5fe-7c00-7a00-8000-000000000211');
  await mode.detectProviderChatProtocol.invoke({ provider_id, model, capability });
  expect(new URL(requests[0].url, 'http://127.0.0.1').pathname).toBe('/api/providers/detect-chat-protocol');
  expect(requests[0].body).toEqual(anonymous);
  expect(requests[0].url).not.toContain('test-secret');
  expect(new URL(requests[1].url, 'http://127.0.0.1').pathname).toBe(`/api/providers/${provider_id}/detect-chat-protocol`);
  expect(requests[1].body).toEqual({ model, capability });
});
