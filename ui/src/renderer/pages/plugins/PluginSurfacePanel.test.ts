import { expect, test } from 'bun:test';
import { parsePluginBridgeRequest } from './PluginSurfacePanel';

test('the one Surface bridge accepts every Unified target', () => {
  for (const target of [
    { target: 'kv', request: { operation: 'get', key: 'x' } },
    { target: 'db', request: { operation: 'query', sql: 'SELECT 1' } },
    { target: 'files', request: { operation: 'list' } },
    { target: 'cache', request: { operation: 'get', key: 'x' } },
    { target: 'actions', action: 'plugin:p/a', input: {} },
    { target: 'host', capability: 'desktop.files.open', input: {} },
    { target: 'config' },
  ]) expect(parsePluginBridgeRequest({ call_id: 'call-1', target })).toEqual({ call_id: 'call-1', target });
});

test('the Surface bridge rejects removed side channels', () => {
  for (const target of ['service', 'host_kv', 'agent_session']) {
    expect(parsePluginBridgeRequest({ call_id: 'call-1', target: { target } })).toBeNull();
  }
});
