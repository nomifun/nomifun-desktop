(() => {
  'use strict';

  if (Object.prototype.hasOwnProperty.call(window, 'nomi')) return;

  const VERSION = '1.0.0';
  const CHALLENGE_EVENT = 'nomifun-plugin-bridge-challenge-v1';
  const HANDSHAKE_EVENT = 'nomifun-plugin-bridge-handshake-v1';
  const CONNECT_EVENT = 'nomifun-plugin-bridge-connect-v1';
  const RESULT_TYPE = 'nomifun-plugin-bridge-result-v1';
  const READY_TIMEOUT_MS = 15_000;
  const CALL_TIMEOUT_MS = 30_000;
  const MAX_JSON_BYTES = 1024 * 1024;
  const MAX_JSON_DEPTH = 64;
  const MAX_BATCH_STATEMENTS = 256;
  const pending = new Map();

  let port;
  let preview = false;
  let closed = false;
  let readyError;
  let challengeNonce;
  let signalReady;
  const ready = new Promise(resolve => { signalReady = resolve; });

  function sdkError(code, message) {
    return Object.assign(new Error(message), { code });
  }

  function isPlainRecord(value) {
    if (Object.prototype.toString.call(value) !== '[object Object]') return false;
    const prototype = Object.getPrototypeOf(value);
    return prototype === null || Object.getPrototypeOf(prototype) === null;
  }

  function assertJsonValue(value, label, depth = 0, seen = new Set()) {
    if (depth > MAX_JSON_DEPTH) throw new TypeError(`${label} exceeds the JSON nesting limit`);
    if (value === null || typeof value === 'string' || typeof value === 'boolean') return;
    if (typeof value === 'number') {
      if (!Number.isFinite(value)) throw new TypeError(`${label} contains a non-finite number`);
      return;
    }
    if (typeof value !== 'object') throw new TypeError(`${label} must be JSON-compatible`);
    if (seen.has(value)) throw new TypeError(`${label} contains a cycle`);
    seen.add(value);
    try {
      if (Object.getOwnPropertySymbols(value).length !== 0) {
        throw new TypeError(`${label} contains symbol properties`);
      }
      if (Array.isArray(value)) {
        if (Object.keys(value).length !== value.length) {
          throw new TypeError(`${label} contains a sparse or extended array`);
        }
        for (let index = 0; index < value.length; index += 1) {
          assertJsonValue(value[index], `${label}[${index}]`, depth + 1, seen);
        }
        return;
      }
      if (!isPlainRecord(value)) throw new TypeError(`${label} must contain plain JSON objects`);
      for (const [key, descriptor] of Object.entries(Object.getOwnPropertyDescriptors(value))) {
        if (key === '__proto__' || key === 'prototype' || key === 'constructor') {
          throw new TypeError(`${label} contains a reserved object key`);
        }
        if (!('value' in descriptor)) throw new TypeError(`${label} contains an accessor`);
        assertJsonValue(descriptor.value, `${label}.${key}`, depth + 1, seen);
      }
    } finally {
      seen.delete(value);
    }
  }

  function assertJsonPayload(value, label) {
    assertJsonValue(value, label);
    const encoded = JSON.stringify(value);
    if (encoded === undefined) throw new TypeError(`${label} must be JSON-compatible`);
    const size = typeof TextEncoder === 'function'
      ? new TextEncoder().encode(encoded).byteLength
      : encoded.length;
    if (size > MAX_JSON_BYTES) throw new RangeError(`${label} exceeds the JSON size limit`);
  }

  function assertString(value, label, maximum, allowEmpty = false) {
    if (typeof value !== 'string' || (!allowEmpty && value.length === 0) || value.length > maximum) {
      throw new TypeError(`${label} must be ${allowEmpty ? '' : 'a non-empty '}string no longer than ${maximum} characters`);
    }
    if (value.includes('\0') || [...value].some(character => /[\u0000-\u001f\u007f]/u.test(character))) {
      throw new TypeError(`${label} contains control characters`);
    }
    return value;
  }

  function assertKey(value, label = 'key') {
    return assertString(value, label, 512);
  }

  function assertPortablePath(value, label, allowEmpty = false) {
    assertString(value, label, 1024, allowEmpty);
    if (value === '' && allowEmpty) return value;
    if (value !== value.normalize('NFC') || value.startsWith('/') || value.endsWith('/')
      || value.includes('\\') || value.includes(':')
      || value.split('/').some(part => part === '' || part === '.' || part === '..')) {
      throw new TypeError(`${label} must be a normalized portable relative path`);
    }
    return value;
  }

  function assertSql(value) {
    if (typeof value !== 'string' || value.length === 0 || value.length > 1024 * 1024
      || value.includes('\0') || value.trim() !== value || value.trim().length === 0) {
      throw new TypeError('sql must be non-empty and trimmed');
    }
    return value;
  }

  function assertSqlParameters(value) {
    if (!Array.isArray(value)) {
      throw new TypeError('SQL parameters must be a JSON array');
    }
    assertJsonPayload(value, 'SQL parameters');
    return value;
  }

  function assertActionIdentity(value) {
    assertString(value, 'action', 320);
    const local = /^[a-z][a-z0-9_-]{0,95}$/u;
    const stable = /^plugin:[a-z0-9][a-z0-9._-]{0,159}\/[a-z][a-z0-9_-]{0,95}$/u;
    if (!local.test(value) && !stable.test(value)) {
      throw new TypeError('action must be a local Action id or plugin:<plugin-id>/<action-id>');
    }
    return value;
  }

  function assertHostAction(value) {
    assertString(value, 'host action', 320);
    if (!/^[a-z][a-z0-9_-]*(?:[./][a-z][a-z0-9_-]*)+$/u.test(value)) {
      throw new TypeError('host action must be a namespaced action id');
    }
    return value;
  }

  function clearPending(error) {
    for (const operation of pending.values()) {
      clearTimeout(operation.timeout);
      operation.reject(error);
    }
    pending.clear();
  }

  function protocolFailure(operation, message) {
    clearTimeout(operation.timeout);
    operation.reject(sdkError('PLUGIN_BRIDGE_PROTOCOL_ERROR', message));
  }

  function base64For(contents, encoding) {
    if (encoding === 'base64') return contents;
    const bytes = new TextEncoder().encode(contents);
    let binary = '';
    for (let index = 0; index < bytes.length; index += 0x8000) {
      binary += String.fromCharCode(...bytes.subarray(index, index + 0x8000));
    }
    return btoa(binary);
  }

  function contentsFromBase64(contents, encoding) {
    if (encoding === 'base64') return contents;
    const binary = atob(contents);
    const bytes = Uint8Array.from(binary, character => character.charCodeAt(0));
    return new TextDecoder('utf-8', { fatal: true }).decode(bytes);
  }

  function wireTarget(method, params) {
    if (method.startsWith('storage.kv.')) {
      const operation = method.slice('storage.kv.'.length);
      return {
        target: 'kv',
        request: {
          operation: operation === 'compareAndSwap' ? 'compare_and_swap' : operation,
          ...params,
        },
      };
    }
    if (method.startsWith('storage.db.')) {
      return { target: 'db', request: { operation: method.slice('storage.db.'.length), ...params } };
    }
    if (method.startsWith('storage.files.')) {
      const operation = method.slice('storage.files.'.length);
      if (operation === 'write') {
        return {
          target: 'files',
          request: {
            operation,
            path: params.path,
            content_base64: base64For(params.contents, params.encoding),
            overwrite: true,
          },
        };
      }
      return { target: 'files', request: { operation, path: params.path } };
    }
    if (method.startsWith('cache.')) {
      return { target: 'cache', request: { operation: method.slice('cache.'.length), ...params } };
    }
    if (method === 'actions.invoke') return { target: 'actions', action: params.action, input: params.input };
    if (method === 'host.invoke') return { target: 'host', capability: params.action, input: params.input };
    if (method === 'config.get') return { target: 'config' };
    throw new TypeError('unsupported Plugin SDK method');
  }

  function decodedResult(method, params, result) {
    const expected = wireTarget(method, params).target;
    if (!isPlainRecord(result) || result.target !== expected) {
      throw new TypeError('Plugin bridge result target is invalid');
    }
    if (expected === 'actions' || expected === 'host') return result.result;
    if (expected === 'config') return result.config;
    const value = result.result;
    if (!isPlainRecord(value)) throw new TypeError('Plugin bridge result is invalid');
    if (method === 'storage.kv.get' || method === 'cache.get') {
      if (value.outcome !== 'value') throw new TypeError('Plugin bridge value result is invalid');
      return value.value ?? null;
    }
    if (method === 'storage.kv.set') {
      if (value.outcome !== 'written') throw new TypeError('Plugin bridge KV write result is invalid');
      return { revision: value.revision };
    }
    if (method === 'storage.kv.compareAndSwap') {
      if (value.outcome !== 'compare_and_swap') throw new TypeError('Plugin bridge CAS result is invalid');
      return { applied: value.applied, revision: value.current_revision ?? null };
    }
    if (method === 'storage.db.query') {
      if (value.outcome !== 'rows') throw new TypeError('Plugin bridge query result is invalid');
      return { rows: value.rows };
    }
    if (method === 'storage.db.execute') {
      if (value.outcome !== 'executed') throw new TypeError('Plugin bridge execute result is invalid');
      return { affectedRows: value.rows_affected };
    }
    if (method === 'storage.db.batch') {
      if (value.outcome !== 'batch' || !Array.isArray(value.results)) {
        throw new TypeError('Plugin bridge batch result is invalid');
      }
      return value.results.map(entry => ({ affectedRows: entry.rows_affected }));
    }
    if (method === 'storage.files.read') {
      if (value.outcome !== 'data' || typeof value.content_base64 !== 'string') {
        throw new TypeError('Plugin bridge file result is invalid');
      }
      return contentsFromBase64(value.content_base64, params.encoding);
    }
    if (method === 'storage.files.list') {
      if (value.outcome !== 'entries') throw new TypeError('Plugin bridge file listing is invalid');
      return value.entries;
    }
    if (method === 'storage.kv.delete' || method === 'storage.files.delete' || method === 'cache.delete') {
      if (value.outcome !== 'deleted') throw new TypeError('Plugin bridge deletion result is invalid');
      return { deleted: value.existed };
    }
    if (method === 'storage.files.write' || method === 'cache.set') {
      if (value.outcome !== (expected === 'files' ? 'written' : 'stored')) {
        throw new TypeError('Plugin bridge write result is invalid');
      }
      return null;
    }
    throw new TypeError('unsupported Plugin SDK result');
  }

  function receive(event) {
    const response = event && event.data;
    if (!response || response.type !== RESULT_TYPE) return;
    if (typeof response.call_id !== 'string') return;
    const operation = pending.get(response.call_id);
    if (!operation) return;
    pending.delete(response.call_id);

    if (response.outcome !== 'success' && response.outcome !== 'failure') {
      protocolFailure(operation, 'Plugin bridge result has no outcome');
      return;
    }
    if (response.outcome === 'success') {
      try {
        assertJsonPayload(response.result, 'Plugin bridge result');
        const value = decodedResult(operation.method, operation.params, response.result);
        assertJsonPayload(value, 'Plugin SDK result');
        clearTimeout(operation.timeout);
        operation.resolve(value);
      } catch (error) {
        protocolFailure(operation, error instanceof Error ? error.message : 'Invalid Plugin bridge result');
      }
      return;
    }
    const failure = response.error;
    if (!isPlainRecord(failure)
      || typeof failure.code !== 'string' || failure.code.length === 0 || failure.code.length > 128
      || typeof failure.message !== 'string' || failure.message.length === 0 || failure.message.length > 4096) {
      protocolFailure(operation, 'Plugin bridge failure is malformed');
      return;
    }
    clearTimeout(operation.timeout);
    operation.reject(sdkError(failure.code, failure.message));
  }

  function receiveWindow(event) {
    if (closed || port || event.source !== window.parent) return;
    const message = event.data;
    if (!isPlainRecord(message) || message.version !== VERSION) return;
    if (message.type === CHALLENGE_EVENT) {
      if (typeof message.nonce !== 'string' || !/^[0-9a-f]{64}$/u.test(message.nonce)) return;
      challengeNonce = message.nonce;
      window.parent.postMessage({ type: HANDSHAKE_EVENT, version: VERSION, nonce: challengeNonce }, '*');
      return;
    }
    if (message.type !== CONNECT_EVENT || message.nonce !== challengeNonce || !challengeNonce) return;
    const transferred = event.ports?.[0];
    if (typeof message.preview !== 'boolean'
      || !transferred || typeof transferred.postMessage !== 'function'
      || typeof transferred.addEventListener !== 'function'
      || typeof transferred.start !== 'function') {
      readyError = sdkError('PLUGIN_BRIDGE_BOOTSTRAP_INVALID', 'Plugin bridge connection is invalid');
      signalReady();
      return;
    }
    preview = message.preview;
    port = transferred;
    port.addEventListener('message', receive);
    port.addEventListener('messageerror', () => {
      clearPending(sdkError('PLUGIN_BRIDGE_MESSAGE_ERROR', 'Plugin bridge message could not be decoded'));
    });
    port.start();
    signalReady();
  }

  async function waitForPort() {
    if (closed) throw sdkError('PLUGIN_BRIDGE_CLOSED', 'Plugin surface closed');
    if (port) return port;
    if (readyError) throw readyError;
    let timeout;
    try {
      await Promise.race([
        ready,
        new Promise((_, reject) => {
          timeout = setTimeout(() => reject(sdkError(
            'PLUGIN_BRIDGE_READY_TIMEOUT',
            'Plugin bridge connection timed out',
          )), READY_TIMEOUT_MS);
        }),
      ]);
    } finally {
      clearTimeout(timeout);
    }
    if (closed) throw sdkError('PLUGIN_BRIDGE_CLOSED', 'Plugin surface closed');
    if (readyError) throw readyError;
    if (!port) throw sdkError('PLUGIN_BRIDGE_UNAVAILABLE', 'Plugin bridge is unavailable');
    return port;
  }

  async function call(method, params) {
    assertString(method, 'method', 128);
    assertJsonPayload(params, 'params');
    const target = wireTarget(method, params);
    assertJsonPayload(target, 'target');
    const bridge = await waitForPort();
    let callId;
    do callId = crypto.randomUUID(); while (pending.has(callId));
    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => {
        pending.delete(callId);
        reject(sdkError(
          'PLUGIN_BRIDGE_CALL_TIMEOUT',
          'Plugin request timed out; its outcome may be unknown',
        ));
      }, CALL_TIMEOUT_MS);
      pending.set(callId, { resolve, reject, timeout, method, params });
      try {
        bridge.postMessage({ call_id: callId, target });
      } catch (error) {
        pending.delete(callId);
        clearTimeout(timeout);
        reject(error);
      }
    });
  }

  const kv = Object.freeze({
    get(key) {
      assertKey(key);
      return call('storage.kv.get', { key });
    },
    set(key, value) {
      assertKey(key);
      assertJsonPayload(value, 'value');
      return call('storage.kv.set', { key, value });
    },
    delete(key) {
      assertKey(key);
      return call('storage.kv.delete', { key });
    },
    compareAndSwap(key, expectedRevision, value) {
      assertKey(key);
      if (expectedRevision !== null
        && (!Number.isSafeInteger(expectedRevision) || expectedRevision < 1)) {
        throw new TypeError('expectedRevision must be null or a positive safe integer');
      }
      const deletes = value === undefined || value === null;
      if (!deletes) assertJsonPayload(value, 'value');
      return call('storage.kv.compareAndSwap', {
        key,
        expected_revision: expectedRevision,
        ...(deletes ? {} : { value }),
      });
    },
  });

  const db = Object.freeze({
    query(sql, parameters = []) {
      assertSql(sql);
      assertSqlParameters(parameters);
      return call('storage.db.query', { sql, parameters });
    },
    execute(sql, parameters = []) {
      assertSql(sql);
      assertSqlParameters(parameters);
      return call('storage.db.execute', { sql, parameters });
    },
    batch(statements) {
      if (!Array.isArray(statements) || statements.length === 0
        || statements.length > MAX_BATCH_STATEMENTS) {
        throw new TypeError(`statements must contain 1-${MAX_BATCH_STATEMENTS} SQL statements`);
      }
      const normalized = statements.map((statement, index) => {
        if (!isPlainRecord(statement)
          || !Object.prototype.hasOwnProperty.call(statement, 'sql')
          || Object.keys(statement).some(key => key !== 'sql' && key !== 'parameters')) {
          throw new TypeError(`statements[${index}] is invalid`);
        }
        const sql = assertSql(statement.sql);
        const parameters = assertSqlParameters(statement.parameters === undefined ? [] : statement.parameters);
        return { sql, parameters };
      });
      return call('storage.db.batch', { statements: normalized });
    },
  });

  function fileOptions(options) {
    if (options === undefined) return { encoding: 'utf8' };
    if (!isPlainRecord(options) || Object.keys(options).some(key => key !== 'encoding')
      || !['utf8', 'base64'].includes(options.encoding)) {
      throw new TypeError('file options must contain encoding utf8 or base64');
    }
    return { encoding: options.encoding };
  }

  function assertFileContents(contents, encoding) {
    if (typeof contents !== 'string') throw new TypeError('contents must be a string');
    const size = typeof TextEncoder === 'function'
      ? new TextEncoder().encode(contents).byteLength
      : contents.length;
    if (size > MAX_JSON_BYTES) throw new RangeError('contents exceeds the file write limit');
    if (encoding === 'base64'
      && !/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/u.test(contents)) {
      throw new TypeError('contents must be canonical base64');
    }
    return contents;
  }

  const files = Object.freeze({
    read(path, options) {
      assertPortablePath(path, 'path');
      return call('storage.files.read', { path, ...fileOptions(options) });
    },
    write(path, contents, options) {
      assertPortablePath(path, 'path');
      const resolved = fileOptions(options);
      assertFileContents(contents, resolved.encoding);
      return call('storage.files.write', { path, contents, ...resolved });
    },
    list(path = '') {
      assertPortablePath(path, 'path', true);
      return call('storage.files.list', { path });
    },
    delete(path) {
      assertPortablePath(path, 'path');
      return call('storage.files.delete', { path });
    },
  });

  const cache = Object.freeze({
    get(key) {
      assertKey(key);
      return call('cache.get', { key });
    },
    set(key, value, options = {}) {
      assertKey(key);
      assertJsonPayload(value, 'value');
      if (!isPlainRecord(options) || Object.keys(options).some(key => key !== 'ttlMs')) {
        throw new TypeError('cache options are invalid');
      }
      const ttlMs = options.ttlMs;
      if (ttlMs !== undefined && (!Number.isSafeInteger(ttlMs) || ttlMs < 1)) {
        throw new TypeError('ttlMs must be a positive safe integer');
      }
      return call('cache.set', {
        key,
        value,
        ...(ttlMs === undefined ? {} : { ttl_ms: ttlMs }),
      });
    },
    delete(key) {
      assertKey(key);
      return call('cache.delete', { key });
    },
  });

  const actions = Object.freeze({
    invoke(action, input = {}) {
      assertActionIdentity(action);
      assertJsonPayload(input, 'input');
      return call('actions.invoke', { action, input });
    },
  });

  const host = Object.freeze({
    invoke(action, input = {}) {
      assertHostAction(action);
      assertJsonPayload(input, 'input');
      return call('host.invoke', { action, input });
    },
  });

  const config = Object.freeze({
    get(...arguments_) {
      if (arguments_.length !== 0) throw new TypeError('config.get does not accept arguments');
      return call('config.get', {});
    },
  });

  const storage = Object.freeze({ kv, db, files });
  const api = {};
  Object.defineProperties(api, {
    version: { value: VERSION, enumerable: true },
    preview: { get: () => preview, enumerable: true },
    storage: { value: storage, enumerable: true },
    cache: { value: cache, enumerable: true },
    actions: { value: actions, enumerable: true },
    host: { value: host, enumerable: true },
    config: { value: config, enumerable: true },
  });
  Object.freeze(api);
  Object.defineProperty(window, 'nomi', {
    value: api,
    enumerable: true,
    configurable: false,
    writable: false,
  });

  window.addEventListener('message', receiveWindow);
  window.addEventListener('pagehide', () => {
    closed = true;
    clearPending(sdkError('PLUGIN_BRIDGE_CLOSED', 'Plugin surface closed'));
    port?.close();
    signalReady();
  }, { once: true });
})();
