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
  return { api: window.nomi.agentSession, storage: window.nomi.storage, host: channel.port2, timers };
}

test('actual SDK exchanges scoped commands and preserves Session error codes', { timeout: 5000 }, async (t) => {
  const { api, host, timers } = runtime(t);
  const received = [];
  host.on('message', (request) => {
    received.push(request);
    const failure = request.target.request.operation === 'cancel';
    host.postMessage({
      type: 'nomifun-plugin-bridge-result-v1', call_id: request.call_id,
      ok: !failure, result: { messages: [] },
      ...(failure ? { error: { code: 'NOMI_CORE_AGENT_SESSION_NOT_FOUND', message: 'Session unavailable' } } : {}),
    });
  });
  assert.deepEqual(await api.observe(), { messages: [] });
  await api.turn({ content: 'hello' }, 'intent-1');
  await assert.rejects(api.cancel(), { code: 'NOMI_CORE_AGENT_SESSION_NOT_FOUND' });
  assert.deepEqual(received.map(value => value.target.request), [
    { operation: 'observe', after_seq: 0, limit: 100 },
    { operation: 'turn', input: { content: 'hello' }, idempotency_key: 'intent-1' },
    { operation: 'cancel' },
  ]);
  for (const request of received) {
    assert.deepEqual(Object.keys(request).sort(), ['call_id', 'target']);
    assert.deepEqual(Object.keys(request.target).sort(), ['request', 'target']);
  }
  assert.equal(timers.size, 0);
});

test('SDK rejects malformed calls locally and never retries an ambiguous turn', { timeout: 5000 }, async (t) => {
  const { api, host, timers } = runtime(t);
  await assert.rejects(api.observe({ limit: 0 }), /Invalid Session history/);
  await assert.rejects(api.turn({}, '字'.repeat(86)), /stable idempotency/);
  assert.equal(timers.size, 0);
  let calls = 0;
  let arrived;
  const arrival = new Promise(resolve => { arrived = resolve; });
  host.on('message', () => { calls++; arrived(); });
  const rejected = assert.rejects(api.turn({ content: 'one intent' }, 'intent-2'), { code: 'PLUGIN_BRIDGE_TIMEOUT' });
  await arrival;
  for (const callback of [...timers.values()]) callback();
  await rejected;
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(calls, 1);
});

test('SDK clears pending request when structured cloning fails', { timeout: 5000 }, async (t) => {
  const { api, timers } = runtime(t);
  await assert.rejects(api.turn({ content: 'hello', invalid: () => {} }, 'intent-3'), { name: 'DataCloneError' });
  assert.equal(timers.size, 0);
});

const prefix = 'nomifun-plugin-agent-session-';

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
function inbox(host) {
  const queued = [];
  const waiting = [];
  host.on('message', value => {
    if (waiting.length) waiting.shift()(value);
    else queued.push(value);
  });
  return { queued, next: () => queued.length ? Promise.resolve(queued.shift()) : new Promise(resolve => waiting.push(resolve)) };
}
function frame(host, subscription_id, seq, fields = {}) {
  host.postMessage({ type: prefix + 'event-v1', subscription_id, seq, kind: 'stream', event: { seq }, ...fields });
}

test('async recovery observes durable history before ACK, without starting a turn', { timeout: 5000 }, async t => {
  const { api, host } = runtime(t);
  const messages = inbox(host);
  let recovered = false;
  const stop = api.subscribe(async value => {
    assert.equal(value.kind, 'resync_required');
    assert.deepEqual(await api.observe(), { messages: ['durable'] });
    recovered = true;
  });
  const subscribe = await messages.next();
  assert.equal(subscribe.type, prefix + 'subscribe-v1');
  frame(host, subscribe.subscription_id, 1, { kind: 'resync_required', reason: 'initial' });
  const observe = await messages.next();
  assert.equal(observe.target.request.operation, 'observe');
  assert.equal(recovered, false);
  assert.equal(messages.queued.length, 0);
  host.postMessage({ type: 'nomifun-plugin-bridge-result-v1', call_id: observe.call_id, ok: true, result: { messages: ['durable'] } });
  assert.equal((await messages.next()).type, prefix + 'ack-v1');
  assert.equal(recovered, true);
  stop();
  assert.equal((await messages.next()).type, prefix + 'unsubscribe-v1');
});

test('serial delivery survives a throwing listener and duplicate functions unsubscribe independently', { timeout: 5000 }, async t => {
  const { api, host } = runtime(t);
  const messages = inbox(host);
  const delivered = [];
  let release;
  const paused = new Promise(resolve => { release = resolve; });
  const callback = async value => {
    delivered.push(value.event.seq);
    if (value.event.seq === 1) await paused;
    if (value.event.seq === 2) throw new Error('plugin error');
  };
  const stop1 = api.subscribe(callback);
  const stop2 = api.subscribe(callback);
  const { subscription_id } = await messages.next();
  stop1();
  frame(host, subscription_id, 1); frame(host, subscription_id, 2); frame(host, subscription_id, 3);
  release();
  for (const seq of [1, 2, 3]) assert.equal((await messages.next()).seq, seq);
  assert.deepEqual(delivered, [1, 2, 3]);
  stop2();
  assert.equal((await messages.next()).type, prefix + 'unsubscribe-v1');
});

test('unsubscribe/resubscribe discards old frames and queued callbacks', { timeout: 5000 }, async t => {
  const { api, host } = runtime(t);
  const messages = inbox(host);
  let entered, release;
  const started = new Promise(resolve => { entered = resolve; });
  const paused = new Promise(resolve => { release = resolve; });
  const old = [];
  const stop = api.subscribe(async value => { old.push(value.event.seq); entered(); await paused; });
  const first = await messages.next();
  frame(host, first.subscription_id, 1);
  await started;
  frame(host, first.subscription_id, 2);
  stop();
  assert.equal((await messages.next()).type, prefix + 'unsubscribe-v1');
  const current = [];
  api.subscribe(value => current.push(value.event.seq));
  const second = await messages.next();
  assert.notEqual(first.subscription_id, second.subscription_id);
  frame(host, first.subscription_id, 3);
  frame(host, second.subscription_id, 4);
  release();
  while ((await messages.next()).seq !== 4) { /* drain old ACKs */ }
  assert.deepEqual(old, [1]);
  assert.deepEqual(current, [4]);
});
