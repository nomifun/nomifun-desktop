import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { MessageChannel } from 'node:worker_threads';
import { webcrypto } from 'node:crypto';

const source = readFileSync(
  new URL('../src/assets/plugin-sdk.js', import.meta.url),
  'utf8',
);
const VERSION = '1.0.0';
const REQUEST_TYPE = 'nomifun-plugin-bridge-request-v1';
const RESULT_TYPE = 'nomifun-plugin-bridge-result-v1';

function harness(t, { connected = true, preview = false } = {}) {
  const channel = new MessageChannel();
  const listeners = new Map();
  const timers = new Map();
  let nextTimer = 0;
  const window = {
    addEventListener(type, listener) {
      const current = listeners.get(type) ?? [];
      current.push(listener);
      listeners.set(type, current);
    },
  };
  if (connected) {
    window.__nomifunPluginBridge = {
      version: VERSION,
      preview,
      port: channel.port1,
    };
  }

  runInNewContext(source, {
    window,
    crypto: webcrypto,
    TextEncoder,
    console: { error() {} },
    setTimeout(callback) {
      const id = ++nextTimer;
      timers.set(id, () => {
        timers.delete(id);
        callback();
      });
      return id;
    },
    clearTimeout(id) {
      timers.delete(id);
    },
  });

  function dispatch(type) {
    for (const listener of listeners.get(type) ?? []) listener({ type });
  }

  function connect(nextPreview = preview) {
    window.__nomifunPluginBridge = {
      version: VERSION,
      preview: nextPreview,
      port: channel.port1,
    };
    dispatch('nomifun-plugin-bridge-ready');
  }

  function fireTimers() {
    for (const callback of [...timers.values()]) callback();
  }

  t.after(() => {
    channel.port1.close();
    channel.port2.close();
    timers.clear();
  });
  return {
    api: window.nomi,
    host: channel.port2,
    timers,
    dispatch,
    connect,
    fireTimers,
  };
}

function success(host, request, result = null) {
  host.postMessage({
    type: RESULT_TYPE,
    version: VERSION,
    call_id: request.call_id,
    ok: true,
    result,
  });
}

function withoutCallId(request) {
  const { call_id, ...rest } = request;
  assert.match(call_id, /^[0-9a-f-]{36}$/u);
  return rest;
}

test('exports one immutable Unified Plugin SDK namespace', t => {
  const { api } = harness(t);
  assert.deepEqual(
    Object.keys(api).sort(),
    ['actions', 'cache', 'config', 'host', 'preview', 'storage', 'version'],
  );
  assert.deepEqual(Object.keys(api.storage).sort(), ['db', 'files', 'kv']);
  assert.deepEqual(
    Object.keys(api.storage.kv).sort(),
    ['compareAndSwap', 'delete', 'get', 'set'],
  );
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
  assert.equal(api.storage.get, undefined);
});

test('all SDK methods use one versioned MessageChannel request shape', { timeout: 5000 }, async t => {
  const { api, host, timers } = harness(t);
  const received = [];
  host.on('message', request => {
    received.push(withoutCallId(request));
    success(host, request);
  });

  await api.storage.kv.get('key');
  await api.storage.kv.set('key', { value: 1 });
  await api.storage.kv.delete('key');
  await api.storage.kv.compareAndSwap('key', 3, { value: 2 });
  await api.storage.kv.compareAndSwap('key', 4);
  await api.storage.db.query('SELECT value FROM things WHERE id = :id', { id: 1 });
  await api.storage.db.execute('INSERT INTO things(value) VALUES (?)', ['hello']);
  await api.storage.db.batch([
    { sql: 'UPDATE things SET value = ? WHERE id = ?', parameters: ['updated', 1] },
    { sql: 'DELETE FROM things WHERE id = ?', parameters: [2] },
  ]);
  await api.storage.files.read('notes/a.txt');
  await api.storage.files.write('notes/a.txt', 'hello\nworld');
  await api.storage.files.write('images/a.bin', 'AAE=', { encoding: 'base64' });
  await api.storage.files.list('notes');
  await api.storage.files.delete('notes/a.txt');
  await api.cache.get('summary');
  await api.cache.set('summary', { count: 2 }, { ttlMs: 5000 });
  await api.cache.delete('summary');
  await api.actions.invoke('refresh', { force: true });
  await api.actions.invoke(
    'plugin:019b0000-0000-7000-8000-000000000001/refresh',
    { force: false },
  );
  await api.host.invoke('desktop.files.open', { path: 'notes/a.txt' });
  await api.config.get();

  const envelope = (method, params) => ({
    type: REQUEST_TYPE,
    version: VERSION,
    method,
    params,
  });
  assert.deepEqual(received, [
    envelope('storage.kv.get', { key: 'key' }),
    envelope('storage.kv.set', { key: 'key', value: { value: 1 } }),
    envelope('storage.kv.delete', { key: 'key' }),
    envelope('storage.kv.compareAndSwap', {
      key: 'key', expected_revision: 3, value: { value: 2 },
    }),
    envelope('storage.kv.compareAndSwap', { key: 'key', expected_revision: 4 }),
    envelope('storage.db.query', {
      sql: 'SELECT value FROM things WHERE id = :id', parameters: { id: 1 },
    }),
    envelope('storage.db.execute', {
      sql: 'INSERT INTO things(value) VALUES (?)', parameters: ['hello'],
    }),
    envelope('storage.db.batch', { statements: [
      { sql: 'UPDATE things SET value = ? WHERE id = ?', parameters: ['updated', 1] },
      { sql: 'DELETE FROM things WHERE id = ?', parameters: [2] },
    ] }),
    envelope('storage.files.read', { path: 'notes/a.txt', encoding: 'utf8' }),
    envelope('storage.files.write', {
      path: 'notes/a.txt', contents: 'hello\nworld', encoding: 'utf8',
    }),
    envelope('storage.files.write', {
      path: 'images/a.bin', contents: 'AAE=', encoding: 'base64',
    }),
    envelope('storage.files.list', { path: 'notes' }),
    envelope('storage.files.delete', { path: 'notes/a.txt' }),
    envelope('cache.get', { key: 'summary' }),
    envelope('cache.set', { key: 'summary', value: { count: 2 }, ttl_ms: 5000 }),
    envelope('cache.delete', { key: 'summary' }),
    envelope('actions.invoke', { action: 'refresh', input: { force: true } }),
    envelope('actions.invoke', {
      action: 'plugin:019b0000-0000-7000-8000-000000000001/refresh',
      input: { force: false },
    }),
    envelope('host.invoke', {
      action: 'desktop.files.open', input: { path: 'notes/a.txt' },
    }),
    envelope('config.get', {}),
  ]);
  assert.equal(timers.size, 0);
});

test('strict inputs fail locally without creating pending calls', async t => {
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
  assert.throws(
    () => api.storage.files.write('file.bin', 'not base64', { encoding: 'base64' }),
    /canonical base64/u,
  );
  assert.throws(() => api.cache.set('x', 1, { ttlMs: 0 }), /ttlMs/u);
  assert.throws(() => api.actions.invoke('plugin:bad'), /local Action id/u);
  assert.throws(() => api.host.invoke('desktop', {}), /namespaced/u);
  assert.throws(() => api.config.get('secret'), /does not accept/u);
  const cyclic = {};
  cyclic.self = cyclic;
  assert.throws(() => api.actions.invoke('refresh', cyclic), /cycle/u);

  await new Promise(resolve => setImmediate(resolve));
  assert.equal(messages, 0);
  assert.equal(timers.size, 0);
});

test('ready and call timeouts reject with typed errors and clean pending state', { timeout: 5000 }, async t => {
  const disconnected = harness(t, { connected: false });
  const readyFailure = assert.rejects(
    disconnected.api.config.get(),
    { code: 'PLUGIN_BRIDGE_READY_TIMEOUT' },
  );
  await Promise.resolve();
  assert.equal(disconnected.timers.size, 1);
  disconnected.fireTimers();
  await readyFailure;
  assert.equal(disconnected.timers.size, 0);

  const connected = harness(t);
  let arrived;
  const arrival = new Promise(resolve => { arrived = resolve; });
  connected.host.on('message', () => arrived());
  const callFailure = assert.rejects(
    connected.api.actions.invoke('mutate', { value: 1 }),
    { code: 'PLUGIN_BRIDGE_CALL_TIMEOUT' },
  );
  await arrival;
  assert.equal(connected.timers.size, 1);
  connected.fireTimers();
  await callFailure;
  assert.equal(connected.timers.size, 0);

  const closing = harness(t);
  let closingArrived;
  const closingArrival = new Promise(resolve => { closingArrived = resolve; });
  closing.host.on('message', () => closingArrived());
  const closedFailure = assert.rejects(
    closing.api.host.invoke('desktop.files.open', { path: 'a.txt' }),
    { code: 'PLUGIN_BRIDGE_CLOSED' },
  );
  await closingArrival;
  closing.dispatch('pagehide');
  await closedFailure;
  assert.equal(closing.timers.size, 0);
  await assert.rejects(
    closing.api.config.get(),
    { code: 'PLUGIN_BRIDGE_CLOSED' },
  );
  assert.equal(closing.timers.size, 0);
});

test('wrong-version and duplicate results cannot settle another operation', { timeout: 5000 }, async t => {
  const { api, host, timers } = harness(t);
  host.on('message', request => {
    host.postMessage({
      type: RESULT_TYPE,
      version: '2.0.0',
      call_id: request.call_id,
      ok: true,
      result: 'wrong',
    });
    success(host, request, { winner: true });
    host.postMessage({
      type: RESULT_TYPE,
      version: VERSION,
      call_id: request.call_id,
      ok: false,
      error: { code: 'DUPLICATE', message: 'must be ignored' },
    });
  });
  assert.deepEqual(
    structuredClone(await api.actions.invoke('refresh')),
    { winner: true },
  );
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(timers.size, 0);
});

test('preview is only a Host bootstrap fact and exposes the identical API and wire', { timeout: 5000 }, async t => {
  const production = harness(t, { preview: false });
  const preview = harness(t, { preview: true });
  const requests = [];
  for (const current of [production, preview]) {
    current.host.on('message', request => {
      requests.push(withoutCallId(request));
      success(current.host, request, { value: 'shared' });
    });
  }

  assert.equal(production.api.preview, false);
  assert.equal(preview.api.preview, true);
  assert.deepEqual(Object.keys(production.api), Object.keys(preview.api));
  assert.deepEqual(Object.keys(production.api.storage), Object.keys(preview.api.storage));
  assert.deepEqual(
    structuredClone(await production.api.storage.kv.get('same')),
    { value: 'shared' },
  );
  assert.deepEqual(
    structuredClone(await preview.api.storage.kv.get('same')),
    { value: 'shared' },
  );
  assert.deepEqual(requests[0], requests[1]);
});
