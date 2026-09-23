import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { MessageChannel } from 'node:worker_threads';
import { webcrypto } from 'node:crypto';

const source = readFileSync(new URL('../src/assets/plugin-sdk.js', import.meta.url), 'utf8');
const VERSION = '1.0.0';
const CHALLENGE = 'nomifun-plugin-bridge-challenge-v1';
const HANDSHAKE = 'nomifun-plugin-bridge-handshake-v1';
const CONNECT = 'nomifun-plugin-bridge-connect-v1';
const RESULT = 'nomifun-plugin-bridge-result-v1';
const NONCE = 'a'.repeat(64);

function harness(t, { connected = true, preview = false } = {}) {
  const channel = new MessageChannel();
  const listeners = new Map();
  const timers = new Map();
  const parentMessages = [];
  let nextTimer = 0;
  const parent = { postMessage(message) { parentMessages.push(structuredClone(message)); } };
  const window = {
    parent,
    addEventListener(type, listener) {
      const current = listeners.get(type) ?? [];
      current.push(listener);
      listeners.set(type, current);
    },
  };

  runInNewContext(source, {
    window,
    crypto: webcrypto,
    TextEncoder,
    TextDecoder,
    btoa,
    atob,
    console: { error() {} },
    setTimeout(callback) {
      const id = ++nextTimer;
      timers.set(id, () => {
        timers.delete(id);
        callback();
      });
      return id;
    },
    clearTimeout(id) { timers.delete(id); },
  });

  function dispatch(type, event = {}) {
    for (const listener of listeners.get(type) ?? []) listener({ type, ...event });
  }

  function challenge(nonce = NONCE, source = parent) {
    dispatch('message', { source, data: { type: CHALLENGE, version: VERSION, nonce } });
  }

  function connect(nextPreview = preview, nonce = NONCE) {
    challenge(nonce);
    assert.deepEqual(parentMessages.at(-1), { type: HANDSHAKE, version: VERSION, nonce });
    dispatch('message', {
      source: parent,
      data: { type: CONNECT, version: VERSION, nonce, preview: nextPreview },
      ports: [channel.port1],
    });
  }

  if (connected) connect(preview);
  t.after(() => {
    channel.port1.close();
    channel.port2.close();
    timers.clear();
  });
  return {
    api: window.nomi,
    host: channel.port2,
    timers,
    parentMessages,
    dispatch,
    challenge,
    connect,
    fireTimers() { for (const callback of [...timers.values()]) callback(); },
  };
}

function resultFor(request) {
  const target = request.target;
  switch (target.target) {
    case 'kv':
      return { target: 'kv', result: {
        get: { outcome: 'value', value: { value: 1 }, revision: 1 },
        set: { outcome: 'written', revision: 2 },
        delete: { outcome: 'deleted', existed: true },
        compare_and_swap: { outcome: 'compare_and_swap', applied: true, current_revision: 3 },
      }[target.request.operation] };
    case 'db': {
      const operation = target.request.operation;
      return { target: 'db', result: operation === 'query'
        ? { outcome: 'rows', rows: [{ value: 'hello' }] }
        : operation === 'execute'
          ? { outcome: 'executed', rows_affected: 1 }
          : { outcome: 'batch', results: target.request.statements.map(() => ({
              outcome: 'executed', rows_affected: 1,
            })) } };
    }
    case 'files':
      return { target: 'files', result: {
        read: { outcome: 'data', content_base64: Buffer.from('hello').toString('base64') },
        write: { outcome: 'written', size_bytes: 5 },
        list: { outcome: 'entries', entries: [{ path: 'notes/a.txt', is_directory: false, size_bytes: 5 }] },
        delete: { outcome: 'deleted', existed: true },
      }[target.request.operation] };
    case 'cache':
      return { target: 'cache', result: {
        get: { outcome: 'value', value: { value: 1 } },
        set: { outcome: 'stored' },
        delete: { outcome: 'deleted', existed: true },
      }[target.request.operation] };
    case 'actions': return { target: 'actions', result: { action: target.action } };
    case 'host': return { target: 'host', result: { capability: target.capability } };
    case 'config': return { target: 'config', config: { label: 'test' } };
    default: throw new Error('unexpected target');
  }
}

function success(host, request, result = resultFor(request)) {
  host.postMessage({ type: RESULT, outcome: 'success', call_id: request.call_id, result });
}

function withoutCallId(request) {
  const { call_id, ...rest } = request;
  assert.match(call_id, /^[0-9a-f-]{36}$/u);
  return rest;
}

test('exports one immutable UI/Service-aligned SDK namespace', t => {
  const { api } = harness(t);
  assert.deepEqual(Object.keys(api).sort(),
    ['actions', 'cache', 'config', 'host', 'preview', 'storage', 'version']);
  assert.deepEqual(Object.keys(api.storage).sort(), ['db', 'files', 'kv']);
  assert.deepEqual(Object.keys(api.storage.kv).sort(),
    ['compareAndSwap', 'delete', 'get', 'set']);
  assert.deepEqual(Object.keys(api.storage.db).sort(), ['batch', 'execute', 'query']);
  assert.deepEqual(Object.keys(api.storage.files).sort(), ['delete', 'list', 'read', 'write']);
  assert.deepEqual(Object.keys(api.cache).sort(), ['delete', 'get', 'set']);
  for (const value of [api, api.storage, api.storage.kv, api.storage.db,
    api.storage.files, api.cache, api.actions, api.host, api.config]) {
    assert.equal(Object.isFrozen(value), true);
  }
  assert.equal(api.version, VERSION);
  assert.equal(api.preview, false);
  assert.equal(api.service, undefined);
});

test('handshake accepts only the exact parent challenge and transferred port', async t => {
  const h = harness(t, { connected: false });
  h.challenge(NONCE, {});
  h.challenge('not-a-nonce');
  assert.equal(h.parentMessages.length, 0);
  h.dispatch('message', {
    source: {},
    data: { type: CONNECT, version: VERSION, nonce: NONCE, preview: false },
    ports: [h.host],
  });
  assert.equal(h.api.preview, false);
  h.connect(true);
  assert.equal(h.api.preview, true);
  h.host.on('message', request => success(h.host, request));
  assert.deepEqual(await h.api.config.get(), { label: 'test' });
});

test('all SDK calls use the Host target DTO and decode its result', { timeout: 5000 }, async t => {
  const { api, host, timers } = harness(t);
  const equalResult = async (operation, expected) => {
    assert.deepEqual(structuredClone(await operation), expected);
  };
  const received = [];
  host.on('message', request => {
    received.push(withoutCallId(request));
    success(host, request);
  });

  await equalResult(api.storage.kv.get('key'), { value: 1 });
  await equalResult(api.storage.kv.set('key', { value: 1 }), { revision: 2 });
  await equalResult(api.storage.kv.delete('key'), { deleted: true });
  await equalResult(api.storage.kv.compareAndSwap('key', 3, { value: 2 }),
    { applied: true, revision: 3 });
  await api.storage.kv.compareAndSwap('key', null);
  await equalResult(api.storage.db.query('SELECT value FROM things WHERE id = ?', [1]),
    { rows: [{ value: 'hello' }] });
  await equalResult(api.storage.db.execute('INSERT INTO things(value) VALUES (?)', ['hello']),
    { affectedRows: 1 });
  await equalResult(api.storage.db.batch([
    { sql: 'UPDATE things SET value = ? WHERE id = ?', parameters: ['updated', 1] },
    { sql: 'DELETE FROM things WHERE id = ?', parameters: [2] },
  ]), [{ affectedRows: 1 }, { affectedRows: 1 }]);
  assert.equal(await api.storage.files.read('notes/a.txt'), 'hello');
  assert.equal(await api.storage.files.read('images/a.bin', { encoding: 'base64' }), 'aGVsbG8=');
  assert.equal(await api.storage.files.write('notes/a.txt', 'hello\nworld'), null);
  assert.equal(await api.storage.files.write('images/a.bin', 'AAE=', { encoding: 'base64' }), null);
  await equalResult(api.storage.files.list('notes'),
    [{ path: 'notes/a.txt', is_directory: false, size_bytes: 5 }]);
  await equalResult(api.storage.files.delete('notes/a.txt'), { deleted: true });
  await equalResult(api.cache.get('summary'), { value: 1 });
  assert.equal(await api.cache.set('summary', { count: 2 }, { ttlMs: 5000 }), null);
  await equalResult(api.cache.delete('summary'), { deleted: true });
  await equalResult(api.actions.invoke('refresh', { force: true }), { action: 'refresh' });
  await equalResult(api.actions.invoke(
    'plugin:019b0000-0000-7000-8000-000000000001/refresh', { force: false }),
  { action: 'plugin:019b0000-0000-7000-8000-000000000001/refresh' });
  await equalResult(api.host.invoke('desktop.files.open', { path: 'notes/a.txt' }),
    { capability: 'desktop.files.open' });
  await equalResult(api.config.get(), { label: 'test' });

  assert.deepEqual(received, [
    { target: { target: 'kv', request: { operation: 'get', key: 'key' } } },
    { target: { target: 'kv', request: { operation: 'set', key: 'key', value: { value: 1 } } } },
    { target: { target: 'kv', request: { operation: 'delete', key: 'key' } } },
    { target: { target: 'kv', request: { operation: 'compare_and_swap', key: 'key', expected_revision: 3, value: { value: 2 } } } },
    { target: { target: 'kv', request: { operation: 'compare_and_swap', key: 'key', expected_revision: null } } },
    { target: { target: 'db', request: { operation: 'query', sql: 'SELECT value FROM things WHERE id = ?', parameters: [1] } } },
    { target: { target: 'db', request: { operation: 'execute', sql: 'INSERT INTO things(value) VALUES (?)', parameters: ['hello'] } } },
    { target: { target: 'db', request: { operation: 'batch', statements: [
      { sql: 'UPDATE things SET value = ? WHERE id = ?', parameters: ['updated', 1] },
      { sql: 'DELETE FROM things WHERE id = ?', parameters: [2] },
    ] } } },
    { target: { target: 'files', request: { operation: 'read', path: 'notes/a.txt' } } },
    { target: { target: 'files', request: { operation: 'read', path: 'images/a.bin' } } },
    { target: { target: 'files', request: { operation: 'write', path: 'notes/a.txt', content_base64: Buffer.from('hello\nworld').toString('base64'), overwrite: true } } },
    { target: { target: 'files', request: { operation: 'write', path: 'images/a.bin', content_base64: 'AAE=', overwrite: true } } },
    { target: { target: 'files', request: { operation: 'list', path: 'notes' } } },
    { target: { target: 'files', request: { operation: 'delete', path: 'notes/a.txt' } } },
    { target: { target: 'cache', request: { operation: 'get', key: 'summary' } } },
    { target: { target: 'cache', request: { operation: 'set', key: 'summary', value: { count: 2 }, ttl_ms: 5000 } } },
    { target: { target: 'cache', request: { operation: 'delete', key: 'summary' } } },
    { target: { target: 'actions', action: 'refresh', input: { force: true } } },
    { target: { target: 'actions', action: 'plugin:019b0000-0000-7000-8000-000000000001/refresh', input: { force: false } } },
    { target: { target: 'host', capability: 'desktop.files.open', input: { path: 'notes/a.txt' } } },
    { target: { target: 'config' } },
  ]);
  assert.equal(timers.size, 0);
});

test('strict inputs fail locally without dispatch', async t => {
  const { api, host, timers } = harness(t);
  let messages = 0;
  host.on('message', () => { messages += 1; });
  assert.throws(() => api.storage.kv.get(''), /key/u);
  assert.throws(() => api.storage.kv.set('x', { value: Number.NaN }), /non-finite/u);
  assert.throws(() => api.storage.kv.compareAndSwap('x', 0, null), /expectedRevision/u);
  assert.throws(() => api.storage.db.query(' SELECT 1'), /trimmed/u);
  assert.throws(() => api.storage.db.execute('SELECT 1', new Date()), /parameters/u);
  assert.throws(() => api.storage.db.batch([]), /statements/u);
  assert.throws(() => api.storage.files.read('../secret'), /portable relative path/u);
  assert.throws(() => api.storage.files.write('file.txt', new Uint8Array([1])), /contents/u);
  assert.throws(() => api.storage.files.write('file.bin', 'not base64', { encoding: 'base64' }), /canonical base64/u);
  assert.throws(() => api.cache.set('x', 1, { ttlMs: 0 }), /ttlMs/u);
  assert.throws(() => api.actions.invoke('plugin:bad'), /local Action id/u);
  assert.throws(() => api.host.invoke('desktop', {}), /namespaced/u);
  assert.throws(() => api.config.get('secret'), /does not accept/u);
  const cyclic = {}; cyclic.self = cyclic;
  assert.throws(() => api.actions.invoke('refresh', cyclic), /cycle/u);
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(messages, 0);
  assert.equal(timers.size, 0);
});

test('ready, call, and pagehide errors settle pending calls', { timeout: 5000 }, async t => {
  const disconnected = harness(t, { connected: false });
  const readyFailure = assert.rejects(disconnected.api.config.get(),
    { code: 'PLUGIN_BRIDGE_READY_TIMEOUT' });
  await Promise.resolve();
  assert.equal(disconnected.timers.size, 1);
  disconnected.fireTimers();
  await readyFailure;
  assert.equal(disconnected.timers.size, 0);

  const connected = harness(t);
  let arrived;
  const arrival = new Promise(resolve => { arrived = resolve; });
  connected.host.on('message', () => arrived());
  const callFailure = assert.rejects(connected.api.actions.invoke('mutate', { value: 1 }),
    { code: 'PLUGIN_BRIDGE_CALL_TIMEOUT' });
  await arrival;
  connected.fireTimers();
  await callFailure;
  assert.equal(connected.timers.size, 0);

  const closing = harness(t);
  let closingArrived;
  const closingArrival = new Promise(resolve => { closingArrived = resolve; });
  closing.host.on('message', () => closingArrived());
  const closedFailure = assert.rejects(closing.api.host.invoke('desktop.files.open', { path: 'a.txt' }),
    { code: 'PLUGIN_BRIDGE_CLOSED' });
  await closingArrival;
  closing.dispatch('pagehide');
  await closedFailure;
  await assert.rejects(closing.api.config.get(), { code: 'PLUGIN_BRIDGE_CLOSED' });
  assert.equal(closing.timers.size, 0);
});

test('wrong result type and duplicate results cannot settle another call', { timeout: 5000 }, async t => {
  const { api, host, timers } = harness(t);
  host.on('message', request => {
    host.postMessage({ type: 'wrong-result', call_id: request.call_id,
      outcome: 'success', result: { target: 'actions', result: 'wrong' } });
    success(host, request, { target: 'actions', result: { winner: true } });
    host.postMessage({ type: RESULT, call_id: request.call_id,
      outcome: 'failure', error: { code: 'DUPLICATE', message: 'ignored' } });
  });
  assert.deepEqual(await api.actions.invoke('refresh'), { winner: true });
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(timers.size, 0);
});

test('a result for another target fails closed', { timeout: 5000 }, async t => {
  const { api, host } = harness(t);
  host.on('message', request => {
    success(host, request, { target: 'config', config: {} });
  });
  await assert.rejects(api.storage.kv.get('key'), { code: 'PLUGIN_BRIDGE_PROTOCOL_ERROR' });
});

test('Preview is only a Host connection fact with the same API and wire', { timeout: 5000 }, async t => {
  const production = harness(t, { preview: false });
  const preview = harness(t, { preview: true });
  const requests = [];
  for (const current of [production, preview]) {
    current.host.on('message', request => {
      requests.push(withoutCallId(request));
      success(current.host, request);
    });
  }
  assert.equal(production.api.preview, false);
  assert.equal(preview.api.preview, true);
  assert.deepEqual(Object.keys(production.api), Object.keys(preview.api));
  assert.deepEqual(await production.api.storage.kv.get('same'), { value: 1 });
  assert.deepEqual(await preview.api.storage.kv.get('same'), { value: 1 });
  assert.deepEqual(requests[0], requests[1]);
});
