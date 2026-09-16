import { expect, test } from 'bun:test';
import { parseBridgeRequest } from './PluginRuntimeSurfacePanel';

test('ordinary plugin Service and storage bridge operations remain available', () => {
  for (const target of [
    { target: 'service', method: 'calculate', payload: { value: 2 } },
    { target: 'host_kv', request: { operation: 'get', key: 'settings' } },
    { target: 'host_kv', request: { operation: 'set', key: 'settings', value: { enabled: true } } },
  ]) {
    expect(parseBridgeRequest({ call_id: 'call-a', target })).toEqual({ call_id: 'call-a', target });
  }
});
test('a plugin frame cannot observe, send to or cancel a host conversation', () => {
  for (const request of [
    { operation: 'observe', after_seq: 0, limit: 50 },
    { operation: 'turn', input: { content: 'hello' }, idempotency_key: 'key' },
    { operation: 'cancel' },
  ]) {
    expect(parseBridgeRequest({ call_id: 'call-a', target: { target: 'agent_session', request } })).toBeNull();
  }
});
