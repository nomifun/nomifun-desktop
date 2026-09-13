#!/usr/bin/env node
// Development preflight in the actual WebKitGTK WebView, NOT native RC evidence.
// Start an isolated Desktop with WEBKIT_INSPECTOR_HTTP_SERVER=127.0.0.1:9232.
// This creates two MiniApps in the explicitly supplied data root. No credentials
// leave the WebView and no external model, browser or computer input is used.
import { readFileSync, mkdirSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { release } from 'node:os';
import { parseArgs } from 'node:util';

const { values } = parseArgs({ options: {
  inspector: { type: 'string' },
  'data-root': { type: 'string' },
  output: { type: 'string' },
  quit: { type: 'boolean', default: false },
} });
if (process.platform !== 'linux' || !values.inspector || !values['data-root'] || !values.output) {
  throw new Error('Linux only: --inspector http://127.0.0.1:9232 --data-root <isolated root> --output <evidence dir>; creates test MiniApps');
}
const endpoint = new URL(values.inspector);
if (endpoint.protocol !== 'http:' || endpoint.hostname !== '127.0.0.1') {
  throw new Error('Inspector must be explicit loopback HTTP');
}
const announcement = JSON.parse(readFileSync(join(values['data-root'], 'port.json'), 'utf8'));
const output = resolve(values.output);
mkdirSync(output, { recursive: true });
const html = await (await fetch(endpoint, { signal: AbortSignal.timeout(5000) })).text();
const socketPath = html.match(/\/socket\/\d+\/\d+\/WebPage/)?.[0];
if (!socketPath) throw new Error('No inspectable WebKitGTK Desktop page');
const ws = new WebSocket(`ws://${endpoint.host}${socketPath}`);
const pending = new Map();
let sequence = 0;
let target;
let readyResolve;
const ready = new Promise((resolveReady) => { readyResolve = resolveReady; });
function rejectPending(message) {
  for (const waiter of pending.values()) {
    clearTimeout(waiter.timer);
    waiter.reject(new Error(message));
  }
  pending.clear();
}
ws.onerror = () => rejectPending('WebKit inspector connection failed');
ws.onclose = () => rejectPending('WebKit inspector connection closed');
ws.onmessage = ({ data }) => {
  const message = JSON.parse(data);
  if (message.method === 'Target.targetCreated' && message.params.targetInfo.type === 'page') {
    target = message.params.targetInfo.targetId;
    readyResolve();
  }
  if (message.method === 'Target.dispatchMessageFromTarget') {
    const inner = JSON.parse(message.params.message);
    const waiter = pending.get(inner.id);
    if (waiter) {
      pending.delete(inner.id);
      clearTimeout(waiter.timer);
      if (inner.error) waiter.reject(new Error(inner.error.message));
      else waiter.resolve(inner.result);
    }
  }
};
function command(method, params = {}) {
  const id = ++sequence;
  return new Promise((resolveCommand, reject) => {
    const timer = setTimeout(() => {
      pending.delete(id);
      reject(new Error(`WebKit command timeout: ${method}`));
    }, 10_000);
    pending.set(id, { resolve: resolveCommand, reject, timer });
    ws.send(JSON.stringify({ id: ++sequence, method: 'Target.sendMessageToTarget', params: {
      targetId: target, message: JSON.stringify({ id, method, params }),
    } }));
  });
}
async function evaluate(expression) {
  const result = await command('Runtime.evaluate', { expression, returnByValue: true });
  if (result.wasThrown) throw new Error('WebKit page evaluation failed');
  return result.result.value;
}
const pause = (ms) => new Promise((done) => setTimeout(done, ms));
async function waitFor(expression, description) {
  const deadline = Date.now() + 15_000;
  while (!(await evaluate(expression))) {
    if (Date.now() > deadline) throw new Error(`WebView timeout: ${description}`);
    await pause(250);
  }
}

// Self-contained page function: fetch uses the product's initialization script
// and local-trust admission. Return summaries, never Surface capabilities.
async function exerciseMiniApps() {
  const results = [];
  const base = `http://127.0.0.1:${window.__backendPort}`;
  async function api(path, body) {
    const response = await fetch(base + path, {
      method: body === undefined ? 'GET' : 'POST',
      headers: body === undefined ? {} : { 'content-type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
      signal: AbortSignal.timeout(120_000),
    });
    const value = await response.json();
    if (!response.ok) throw new Error(`${path}: HTTP ${response.status}, ${value.code ?? value.error?.code ?? 'unknown'}`);
    return value.data;
  }
  const runtime = await api('/api/javascript-runtime/status');
  if (!runtime.selected) throw new Error('No selected Node; configure Runtime Manager first');
  await api('/api/plugins');
  await api('/api/agent-preset-templates?source=official');
  for (const kind of ['ui_only', 'service']) {
    const library = await api('/api/plugins/runtimes');
    let current = await api('/api/plugins/runtimes/projects', {
      expected_library_revision: library.library_revision,
      display_name: `Linux WebKit ${kind} ${Date.now()}`,
      description: 'Isolated Linux development smoke', kind,
    });
    const id = current.miniapp.miniapp_id;
    const path = `/api/plugins/runtimes/${encodeURIComponent(id)}`;
    current = await api(`${path}/build`, {
      miniapp_id: id, expected_product_revision: current.miniapp.product_revision,
      project_id: current.project_id, expected_project_revision: current.project_revision,
      expected_build_generation: current.build_generation,
      expected_source_snapshot_digest: current.source_snapshot_digest,
      expected_dependency_lock_digest: current.dependency_lock_digest,
    });
    if (!current.ready?.release) throw new Error(`${kind}: Build produced no Ready Release`);
    if (kind === 'service') {
      current = await api(`${path}/test`, {
        miniapp_id: id, expected_product_revision: current.miniapp.product_revision,
        expected_pointer_revision: current.miniapp.releases.pointer_revision,
        project_id: current.project_id, expected_project_revision: current.project_revision,
        expected_build_generation: current.build_generation,
        release_id: current.ready.release.release_id,
        expected_release_digest: current.ready.release.release_digest,
        expected_config_revision: current.config.config_revision,
        expected_credential_bindings_revision: current.credential_bindings_revision,
        resolved_test_input_digest: 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855',
      });
      if (current.ready.test?.status !== 'passed') throw new Error(`Service Test: ${current.ready.test?.error_code}`);
    }
    current = await api(`${path}/publish`, {
      miniapp_id: id, expected_product_revision: current.miniapp.product_revision,
      expected_pointer_revision: current.miniapp.releases.pointer_revision,
      expected_active_release_epoch: current.miniapp.releases.active_release_epoch,
      ready_release_id: current.ready.release.release_id,
      expected_ready_release_digest: current.ready.release.release_digest,
      ...(kind === 'service' ? { expected_service_test_receipt_id: current.ready.test.receipt_id } : {}),
      acknowledge_test_warning: false,
    });
    if (current.miniapp.lifecycle === 'disabled') {
      current = await api(`${path}/enabled`, {
        miniapp_id: id, expected_product_revision: current.miniapp.product_revision,
        expected_pointer_revision: current.miniapp.releases.pointer_revision,
        expected_active_release_digest: current.miniapp.releases.active.release_digest,
        enabled: true,
      });
    }
    const surface = await api(`${path}/surface/open`, { miniapp_id: id });
    if (surface.release_id !== current.miniapp.releases.active.release_id) throw new Error('Surface Release mismatch');
    const assetPath = `${path}/surface/assets/${encodeURIComponent(surface.surface_capability)}/${surface.active_release_epoch}/${encodeURIComponent(surface.expected_release_digest)}/${surface.ui_entrypoint.split('/').map(encodeURIComponent).join('/')}`;
    const asset = await fetch(base + assetPath);
    if (!asset.ok || !(await asset.text()).includes('Linux WebKit')) throw new Error(`${kind}: Surface HTML missing`);
    const closed = await api(`${path}/surface/close`, {
      miniapp_id: id, surface_session_id: surface.surface_session_id,
      surface_capability: surface.surface_capability,
    });
    if (closed !== true || (await fetch(base + assetPath)).status !== 404) throw new Error(`${kind}: Surface capability was not revoked`);
    if (kind === 'service') {
      for (const running of [true, false]) {
        current = await api(`${path}/service/running`, {
          miniapp_id: id, expected_product_revision: current.miniapp.product_revision,
          expected_pointer_revision: current.miniapp.releases.pointer_revision,
          expected_active_release_epoch: current.miniapp.releases.active_release_epoch,
          expected_active_release_digest: current.miniapp.releases.active.release_digest, running,
        });
      }
    }
    results.push({ kind, miniapp_id: id, display_name: current.miniapp.display_name,
      build: 'ready', publish: 'active', surface: 'opened-and-closed',
      ...(kind === 'service' ? { test: 'passed', service: 'started-and-stopped' } : {}) });
  }
  return results;
}

const report = { evidence_kind: 'development-preflight', wsl: /microsoft|wsl/i.test(release()), checks: [] };
let readyTimer;
try {
  await Promise.race([ready, new Promise((_, reject) => {
    readyTimer = setTimeout(() => reject(new Error('WebKit page target timeout')), 5000);
  })]);
  clearTimeout(readyTimer);
  if (await evaluate('window.__backendPort') !== announcement.port) throw new Error('Inspector is not the Desktop serving the supplied isolated data root');
  report.webview_origin = await evaluate('location.origin');
  for (const route of ['/agent', '/plugins', '/plugins']) {
    await evaluate(`location.hash = ${JSON.stringify(route)}`);
    await pause(1500);
    const page = await evaluate('({hash:location.hash,text:document.body.innerText})');
    if (page.hash !== `#${route}` || page.text.length < 100 || /unexpected application error/i.test(page.text)) throw new Error(`Page failed: ${route}`);
    report.checks.push({ route, hash: page.hash, rendered_text_length: page.text.length });
  }
  await evaluate(`window.__linuxPreflight = null; (${exerciseMiniApps.toString()})().then(value => { window.__linuxPreflight = {ok:true,value}; }, error => { window.__linuxPreflight = {ok:false,error:error.message}; }); void 0`);
  const deadline = Date.now() + 180_000;
  let result;
  while (!(result = await evaluate('window.__linuxPreflight'))) {
    if (Date.now() > deadline) throw new Error('MiniApp product preflight deadline exceeded');
    await pause(250);
  }
  await evaluate('delete window.__linuxPreflight');
  if (!result.ok) throw new Error(result.error);
  report.miniapps = result.value;
  for (const miniapp of report.miniapps) {
    await evaluate(`location.hash = ${JSON.stringify(`/plugins/run/${miniapp.miniapp_id}`)}`);
    const openButton = `Array.from(document.querySelectorAll('button')).find(button => /^(Open Surface|打开 Surface):/.test(button.getAttribute('aria-label') || '') && button.getAttribute('aria-label').endsWith(${JSON.stringify(`: ${miniapp.display_name}`)}) && !button.disabled)`;
    await waitFor(`Boolean(${openButton})`, 'MiniApp Open Surface button (en-US/zh-CN)');
    await evaluate(`(${openButton}).click(); void 0`);
    await waitFor(`(() => {
      const section = document.querySelector('section[aria-labelledby="miniapp-surface-title"]');
      const frame = section?.querySelector('iframe');
      return frame?.getAttribute('sandbox') === 'allow-scripts allow-forms' &&
        section.getAttribute('aria-busy') !== 'true' && !section.querySelector('[role="alert"]');
    })()`, 'sandboxed Surface iframe load');
    const closeButton = `Array.from(document.querySelectorAll('button')).find(button => /^(Close Surface|关闭 Surface):/.test(button.getAttribute('aria-label') || '') && !button.disabled)`;
    await evaluate(`(${closeButton}).click(); void 0`);
    await waitFor(`!document.querySelector('section[aria-labelledby="miniapp-surface-title"] iframe')`, 'Surface iframe unmount');
    miniapp.webview_surface = 'loaded-and-closed-via-ui';
  }
  await evaluate('location.hash = "/plugins"; void 0');
  report.status = 'preflight-pass';
  if (values.quit) {
    // Schedule after this evaluation response; the normal Tauri exit path owns
    // backend/process cleanup. Verify actual process/port removal separately.
    await evaluate('setTimeout(() => window.__TAURI_INTERNALS__.invoke("plugin:process|exit", {code:0}), 100); void 0');
    report.shutdown = 'normal-exit-requested';
  }
} catch (error) {
  report.status = 'preflight-fail';
  report.error = error.message;
  process.exitCode = 1;
} finally {
  clearTimeout(readyTimer);
  ws.close();
  writeFileSync(join(output, 'linux-webkit-preflight.json'), JSON.stringify(report, null, 2) + '\n');
  console.log(JSON.stringify(report));
}
