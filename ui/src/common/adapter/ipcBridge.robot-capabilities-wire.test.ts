import { afterEach, expect, test } from 'bun:test';
import { robot, type IApiRobotCapabilities } from './ipcBridge';

const realFetch = globalThis.fetch;
afterEach(() => { globalThis.fetch = realFetch; });
const data: IApiRobotCapabilities = { robot_id: 'a:b/c', connected: true, revision: 9,
  profile: { tool_overrides: {}, presets: [] }, tools: [] };

test('robot capabilities use encoded device identity and a revisioned profile without hardware calls', async () => {
  const calls: Array<{ url: string; method: string; body: unknown }> = [];
  globalThis.fetch = (async (input: string | URL | Request, init?: RequestInit) => {
    calls.push({ url: String(input), method: init?.method ?? 'GET',
      body: typeof init?.body === 'string' ? JSON.parse(init.body) : undefined });
    return new Response(JSON.stringify({ success: true, data }), { headers: { 'Content-Type': 'application/json' } });
  }) as typeof fetch;
  expect(await robot.getCapabilities.invoke({ robot_id: data.robot_id })).toEqual(data);
  await robot.setCapabilities.invoke({ robot_id: data.robot_id, expected_revision: 9, profile: data.profile });
  await robot.refreshCapabilities.invoke({ robot_id: data.robot_id });
  expect(calls.map((call) => ({ method: call.method, path: new URL(call.url, 'http://localhost').pathname, body: call.body }))).toEqual([
    { method: 'GET', path: '/api/robots/a%3Ab%2Fc/capabilities', body: undefined },
    { method: 'PUT', path: '/api/robots/a%3Ab%2Fc/capabilities', body: { expected_revision: 9, profile: data.profile } },
    { method: 'POST', path: '/api/robots/a%3Ab%2Fc/capabilities/refresh', body: {} },
  ]);
});
