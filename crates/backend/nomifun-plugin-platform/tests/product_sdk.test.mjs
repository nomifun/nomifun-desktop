// Run in Node so DOM test preloads cannot replace the real MessageChannel.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { MessageChannel } from 'node:worker_threads';
import { webcrypto } from 'node:crypto';

const sdk = readFileSync(new URL('../src/assets/product-sdk.js', import.meta.url), 'utf8');

function runtime(t) {
  const channel = new MessageChannel();
  let timerId = 0;
  const timers = new Map();
  const window = { __nomifunPluginBridge: channel.port1, addEventListener() {} };
  runInNewContext(sdk, {
    window, crypto: webcrypto, TextEncoder, console: { error() {} },
    setTimeout(callback) { timers.set(++timerId, callback); return timerId; },
    clearTimeout(id) { timers.delete(id); },
  });
  t.after(() => { channel.port1.close(); channel.port2.close(); timers.clear(); });
  assert.equal(window.nomi.agentSession, undefined);
  return { api: window.nomi.service, storage: window.nomi.storage, host: channel.port2, timers };
}

test('ordinary Service SDK exchanges calls and preserves host error codes without Session authority', { timeout: 5000 }, async t => {
  const { api, host, timers } = runtime(t);
  const received = [];
  host.on('message', request => {
    received.push(request);
    const failure = request.target.method === 'fail';
    host.postMessage({ type: 'nomifun-plugin-bridge-result-v1', call_id: request.call_id,
      ok: !failure, result: { value: 7 },
      ...(failure ? { error: { code: 'SERVICE_UNAVAILABLE', message: 'Service unavailable' } } : {}) });
  });
  assert.deepEqual(await api.invoke('echo', { value: 7 }), { value: 7 });
  await assert.rejects(api.invoke('fail'), { code: 'SERVICE_UNAVAILABLE' });
  assert.deepEqual(received.map(value => value.target), [
    { target: 'service', method: 'echo', payload: { value: 7 } },
    { target: 'service', method: 'fail', payload: {} },
  ]);
  assert.equal(timers.size, 0);
});

test('SDK rejects invalid Service calls locally and never retries ambiguous effects', { timeout: 5000 }, async t => {
  const { api, host, timers } = runtime(t);
  await assert.rejects(api.invoke(''), /Invalid service method/);
  await assert.rejects(api.invoke('x'.repeat(257)), /Invalid service method/);
  assert.equal(timers.size, 0);
  let calls = 0;
  let arrived;
  const arrival = new Promise(resolve => { arrived = resolve; });
  host.on('message', () => { calls++; arrived(); });
  const rejected = assert.rejects(api.invoke('mutate', { value: 1 }), { code: 'PLUGIN_BRIDGE_TIMEOUT' });
  await arrival;
  for (const callback of [...timers.values()]) callback();
  await rejected;
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(calls, 1);
});

test('SDK clears pending request when structured cloning fails', { timeout: 5000 }, async t => {
  const { api, timers } = runtime(t);
  await assert.rejects(api.invoke('echo', { invalid: () => {} }), { name: 'DataCloneError' });
  assert.equal(timers.size, 0);
});

test('versioned storage uses existing KV wire, preserves tombstones and reports conflicts without retry', { timeout: 5000 }, async t => {
  const { storage, host } = runtime(t);
  const calls = [];
  const responses = [
    { outcome: 'value' },
    { outcome: 'compare_and_swap', applied: true, current_revision: 1 },
    { outcome: 'value', value: { text: 'saved' }, revision: 1 },
    { outcome: 'compare_and_swap', applied: false, current_revision: 2 },
    { outcome: 'compare_and_swap', applied: true, current_revision: 3 },
    { outcome: 'value', revision: 3 },
  ];
  host.on('message', request => {
    calls.push(request.target);
    host.postMessage({ type: 'nomifun-plugin-bridge-result-v1', call_id: request.call_id, ok: true, result: responses.shift() });
  });
  // SDK projections are constructed in the VM realm; clone into this realm
  // so strict prototype comparison does not reject equivalent wire values.
  assert.deepEqual(structuredClone(await storage.read('draft')), { value: null, revision: null });
  assert.deepEqual(structuredClone(await storage.compareAndSwap('draft', null, { text: 'saved' })), { applied: true, revision: 1 });
  assert.deepEqual(structuredClone(await storage.read('draft')), { value: { text: 'saved' }, revision: 1 });
  assert.deepEqual(structuredClone(await storage.compareAndSwap('draft', 1, { text: 'stale' })), { applied: false, revision: 2 });
  assert.deepEqual(structuredClone(await storage.compareAndSwap('draft', 2)), { applied: true, revision: 3 });
  assert.deepEqual(structuredClone(await storage.read('draft')), { value: null, revision: 3 });
  assert.deepEqual(calls[1], { target: 'host_kv', request: { operation: 'compare_and_swap', key: 'draft', value: { text: 'saved' } } });
  assert.deepEqual(calls[4], { target: 'host_kv', request: { operation: 'compare_and_swap', key: 'draft', expected_revision: 2 } });
  assert.equal(calls.length, 6);
});

test('versioned storage rejects unsafe revisions and does not accept malformed success receipts', { timeout: 5000 }, async t => {
  const { storage, host, timers } = runtime(t);
  for (const revision of [undefined, 0, -1, 1.5, Number.MAX_SAFE_INTEGER + 1, '1']) {
    await assert.rejects(storage.compareAndSwap('draft', revision, {}));
  }
  assert.equal(timers.size, 0);
  const responses = [
    { outcome: 'value', value: 'data' },
    { outcome: 'value', revision: Number.MAX_SAFE_INTEGER + 1 },
    { outcome: 'compare_and_swap', applied: true },
  ];
  host.on('message', request => host.postMessage({ type: 'nomifun-plugin-bridge-result-v1', call_id: request.call_id, ok: true, result: responses.shift() }));
  await assert.rejects(storage.read('draft'), /no revision/);
  await assert.rejects(storage.read('draft'), /Invalid storage revision/);
  await assert.rejects(storage.compareAndSwap('draft', null, {}), /no revision/);
});
