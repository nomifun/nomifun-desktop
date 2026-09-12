(() => {
  if (window.nomi) return;
  const pending = new Map();
  let port;
  let resolveReady;
  const ready = new Promise(resolve => { resolveReady = resolve; });
  function connect() {
    if (port || !window.__nomifunMiniAppBridge) return;
    port = window.__nomifunMiniAppBridge;
    port.addEventListener('message', event => {
      const result = event.data;
      if (!result || result.type !== 'nomifun-miniapp-bridge-result-v1') return;
      const operation = pending.get(result.call_id);
      if (!operation) return;
      pending.delete(result.call_id);
      clearTimeout(operation.timeout);
      if (result.ok) operation.resolve(result.result);
      else operation.reject(new Error(result.error?.message || 'Storage is unavailable'));
    });
    port.start();
    resolveReady();
  }
  window.addEventListener('nomifun-miniapp-bridge-ready', connect);
  connect();
  async function call(request) {
    if (typeof request.key !== 'string' || !request.key || request.key.length > 256) throw new Error('Invalid storage key');
    let timer;
    try {
      await Promise.race([ready, new Promise((_, reject) => { timer = setTimeout(() => reject(new Error('Storage connection timed out')), 15000); })]);
    } finally { clearTimeout(timer); }
    const call_id = crypto.randomUUID();
    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => { pending.delete(call_id); reject(new Error('Storage operation timed out')); }, 15000);
      pending.set(call_id, { resolve, reject, timeout });
      port.postMessage({ call_id, target: { target: 'host_kv', request } });
    });
  }
  Object.defineProperty(window, 'nomi', { value: Object.freeze({ storage: Object.freeze({
    async get(key) { const result = await call({ operation: 'get', key }); return result.value ?? null; },
    async set(key, value) { await call({ operation: 'set', key, value }); },
    async delete(key) { await call({ operation: 'delete', key }); }
  }) }), writable: false });
})();
