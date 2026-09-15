(() => {
  if (window.nomi) return;
  const pending = new Map();
  let port;
  let resolveReady;
  const ready = new Promise(resolve => { resolveReady = resolve; });
  function connect() {
    if (port || !window.__nomifunPluginBridge) return;
    port = window.__nomifunPluginBridge;
    port.addEventListener('message', event => {
      const result = event.data;
      if (!result || result.type !== 'nomifun-plugin-bridge-result-v1') return;
      const operation = pending.get(result.call_id);
      if (!operation) return;
      pending.delete(result.call_id);
      clearTimeout(operation.timeout);
      if (result.ok) operation.resolve(result.result);
      else operation.reject(Object.assign(new Error(result.error?.message || 'Plugin request failed'), { code: result.error?.code }));
    });
    port.start();
    resolveReady();
  }
  window.addEventListener('nomifun-plugin-bridge-ready', connect);
  connect();
  async function callTarget(target) {
    let timer;
    try {
      await Promise.race([ready, new Promise((_, reject) => { timer = setTimeout(() => reject(new Error('Plugin bridge connection timed out')), 15000); })]);
    } finally { clearTimeout(timer); }
    const call_id = crypto.randomUUID();
    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => { pending.delete(call_id); reject(Object.assign(new Error('Plugin request timed out; outcome may be unknown'), { code: 'PLUGIN_BRIDGE_TIMEOUT' })); }, 15000);
      pending.set(call_id, { resolve, reject, timeout });
      try { port.postMessage({ call_id, target }); }
      catch (error) { pending.delete(call_id); clearTimeout(timeout); reject(error); }
    });
  }
  function call(request) {
    if (typeof request.key !== 'string' || !request.key || request.key.length > 256) return Promise.reject(new Error('Invalid storage key'));
    return callTarget({ target: 'host_kv', request });
  }
  function checkedRevision(value) {
    if (value === null || value === undefined) return null;
    if (!Number.isSafeInteger(value) || value < 1) throw new Error('Invalid storage revision');
    return value;
  }
  Object.defineProperty(window, 'nomi', { value: Object.freeze({ preview: false, storage: Object.freeze({
    async get(key) { const result = await call({ operation: 'get', key }); return result.value ?? null; },
    // Reuse the existing versioned KV contract. A tombstone retains its
    // revision; null revision means this key has never existed.
    async read(key) {
      const result = await call({ operation: 'get', key });
      if (result?.outcome !== 'value') throw new Error('Invalid storage read response');
      const revision = checkedRevision(result.revision);
      if (result.value != null && revision === null) throw new Error('Storage value has no revision');
      return { value: result.value ?? null, revision };
    },
    // Omit value (or pass null) to delete with a revision guard. No retries:
    // after a timeout, read/reconcile explicitly before another write.
    async compareAndSwap(key, expectedRevision, value) {
      if (expectedRevision === undefined) throw new Error('An expected storage revision or null is required');
      const revision = checkedRevision(expectedRevision);
      const result = await call({ operation: 'compare_and_swap', key,
        ...(revision === null ? {} : { expected_revision: revision }),
        ...(value === undefined || value === null ? {} : { value }) });
      if (result?.outcome !== 'compare_and_swap' || typeof result.applied !== 'boolean') throw new Error('Invalid storage CAS response');
      const current = checkedRevision(result.current_revision);
      if (result.applied && value != null && current === null) throw new Error('Storage write has no revision');
      return { applied: result.applied, revision: current };
    },
    async set(key, value) { await call({ operation: 'set', key, value }); },
    async delete(key) { await call({ operation: 'delete', key }); }
  }), service: Object.freeze({
    invoke(method, payload = {}) {
      if (typeof method !== 'string' || !method || method.length > 256) return Promise.reject(new Error('Invalid service method'));
      return callTarget({ target: 'service', method, payload });
    }
  }), agentSession: Object.freeze({
    // The host UI must explicitly bind this Surface to a Session first.
    // Never auto-retry mutations; retain the same caller-supplied key when
    // reconciling an ambiguous turn result after a view reconnect.
    observe({ after_seq = 0, limit = 100 } = {}) {
      if (!Number.isSafeInteger(after_seq) || after_seq < 0 ||
          !Number.isSafeInteger(limit) || limit < 1 || limit > 200) {
        return Promise.reject(new Error('Invalid Session history page'));
      }
      return callTarget({ target: 'agent_session', request: { operation: 'observe', after_seq, limit } });
    },
    turn(input, idempotency_key) {
      if (!input || typeof input !== 'object' || Array.isArray(input) ||
          typeof idempotency_key !== 'string' || !idempotency_key.trim() ||
          new TextEncoder().encode(idempotency_key).length > 256) {
        return Promise.reject(new Error('A Session turn needs an input object and a stable idempotency key'));
      }
      return callTarget({ target: 'agent_session', request: { operation: 'turn', input, idempotency_key } });
    },
    cancel() {
      return callTarget({ target: 'agent_session', request: { operation: 'cancel' } });
    }
  }) }), writable: false });
})();
