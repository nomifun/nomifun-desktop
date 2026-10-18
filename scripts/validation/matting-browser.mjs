/**
 * Exercise the real model transport and ONNX worker without opening the app or user data.
 *
 * Required: NOMIFUN_MATTING_MODEL_PATH points to the approved model in matting-model.json.
 * Optional: NOMIFUN_LIVE_PLAYWRIGHT_MODULE points to an installed Playwright entry module;
 * otherwise an installed playwright/playwright-core package is used. Edge is the default
 * browser; NOMIFUN_MATTING_BROWSER_CHANNEL / NOMIFUN_MATTING_BROWSER_EXECUTABLE override it.
 * NOMIFUN_MATTING_BROWSER_REPORT optionally saves the small, synthetic-data-only JSON report.
 */
import assert from 'node:assert/strict';
import { createHash, randomBytes } from 'node:crypto';
import { readFile, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const manifest = JSON.parse(await readFile(resolve(root, 'crates/backend/nomifun-companion/matting-model.json'), 'utf8'));
let browser;
let vite;
let backend;
let stage = 'INPUTS';
const requestLog = [];
const report = { model: { id: manifest.id, size: manifest.size, sha256: manifest.sha256 }, modes: [] };

const noCacheStorage = `Object.defineProperty(globalThis, 'caches', { configurable: true,
  get() { throw new Error('Cache Storage disabled by matting browser regression'); } });`;
const noWebCrypto = `if (globalThis.crypto) Object.defineProperty(globalThis.crypto, 'subtle', { configurable: true,
  get() { throw new Error('Web Crypto disabled by matting browser regression'); } });`;
const harnessPath = '/__nomifun_matting_browser';
const harness = `<!doctype html><html><head><title>Matting browser regression</title></head>
<body><main>Disposable matting regression, synthetic image only.</main><script type="module">
import { ensureMattingModel } from '/src/renderer/services/matting/modelCache.ts';
import { MattingClient } from '/src/renderer/services/matting/mattingClient.ts';
window.__mattingBrowser = {
  async acquire(verifyDigest) {
    const model = await ensureMattingModel();
    if (!verifyDigest) return { size: model.size };
    const digest = await crypto.subtle.digest('SHA-256', await model.arrayBuffer());
    return { size: model.size, sha256: [...new Uint8Array(digest)].map(v => v.toString(16).padStart(2, '0')).join('') };
  },
  async infer() {
    const canvas = document.createElement('canvas');
    canvas.width = canvas.height = 256;
    const context = canvas.getContext('2d');
    context.fillStyle = '#dddddd'; context.fillRect(0, 0, 256, 256);
    context.fillStyle = '#cc3333'; context.fillRect(80, 40, 96, 176);
    const blob = await new Promise(resolve => canvas.toBlob(resolve, 'image/png'));
    const file = new File([blob], 'synthetic-matting.png', { type: 'image/png' });
    const client = new MattingClient();
    const messages = [];
    const started = performance.now();
    try {
      const outcome = await new Promise((resolve, reject) => {
        const timeout = setTimeout(() => reject(new Error('Matting client did not finish')), 45000);
        client.run({ id: 1, file, mode: 'auto', tolerance: 11 }, message => {
          messages.push(message.type === 'done' ? { type: message.type, method: message.method, bytes: message.blob.size }
            : message.type === 'fallback' ? { type: message.type, reason: message.reason, detail: message.detail }
            : message.type === 'error' ? { type: message.type, message: message.message }
            : { type: message.type, phase: message.phase });
          if (message.type === 'error' || message.type === 'done') {
            clearTimeout(timeout);
            resolve({ messages, durationMs: Math.round(performance.now() - started) });
          }
        });
      });
      return outcome;
    } finally { client.dispose(); }
  },
};
</script></body></html>`;

async function loadPlaywright() {
  if (process.env.NOMIFUN_LIVE_PLAYWRIGHT_MODULE) {
    return import(pathToFileURL(resolve(process.env.NOMIFUN_LIVE_PLAYWRIGHT_MODULE)).href);
  }
  for (const name of ['playwright', 'playwright-core']) {
    try { return await import(name); } catch (error) {
      if (error.code !== 'ERR_MODULE_NOT_FOUND') throw error;
    }
  }
  throw new Error('Set NOMIFUN_LIVE_PLAYWRIGHT_MODULE to an existing Playwright module');
}

async function startBackend(model) {
  const secret = randomBytes(24).toString('hex');
  const cookie = randomBytes(24).toString('hex');
  let rendererOrigin;
  backend = createServer((request, response) => {
    const url = new URL(request.url, 'http://127.0.0.1');
    const origin = request.headers.origin;
    // Match production desktop CORS: an exact permitted origin, explicit methods,
    // wildcard allowed request headers, and deliberately no Allow-Credentials.
    if (origin === rendererOrigin) {
      response.setHeader('Access-Control-Allow-Origin', origin);
      response.setHeader('Access-Control-Allow-Methods', 'GET,POST,PUT,PATCH,DELETE,OPTIONS');
      response.setHeader('Access-Control-Allow-Headers', '*');
      response.setHeader('Vary', 'Origin');
    }
    if (request.method === 'OPTIONS') {
      requestLog.push({ method: 'OPTIONS', approvedOrigin: origin === rendererOrigin });
      response.writeHead(origin === rendererOrigin ? 204 : 403).end();
      return;
    }
    if (request.method !== 'GET' || url.pathname !== '/api/companion/matting-model' || url.searchParams.get('sha256') !== manifest.sha256) {
      response.writeHead(404).end();
      return;
    }
    const localTrust = request.headers['x-nomi-local-trust'] === secret;
    const sessionCookie = (request.headers.cookie ?? '').split(';').some(value => value.trim() === `matting_fixture_session=${cookie}`);
    requestLog.push({ method: 'GET', localTrust, sessionCookie, approvedOrigin: origin === rendererOrigin });
    if (!localTrust && !sessionCookie) {
      response.writeHead(403, { 'Content-Type': 'application/json' }).end(JSON.stringify({ success: false, error: 'Fixture authentication required' }));
      return;
    }
    response.writeHead(200, { 'Content-Type': 'application/octet-stream', 'Content-Length': model.length, 'Cache-Control': 'no-store' });
    response.end(model);
  });
  await new Promise((accept, reject) => {
    backend.once('error', reject);
    backend.listen(0, '127.0.0.1', accept);
  });
  const address = backend.address();
  assert(address && typeof address !== 'string');
  return { port: address.port, secret, cookie, setRendererOrigin(value) { rendererOrigin = value; } };
}

async function startRenderer(backendPort) {
  const { createServer: createViteServer } = await import(pathToFileURL(resolve(root, 'ui/node_modules/vite/dist/node/index.js')).href);
  vite = await createViteServer({
    configFile: resolve(root, 'ui/vite.config.ts'), mode: 'development', logLevel: 'silent',
    plugins: [{
      name: 'nomifun-matting-browser-fixture',
      enforce: 'pre',
      configureServer(server) {
        server.middlewares.use((request, response, next) => {
          if (request.url?.split('?')[0] !== harnessPath) return next();
          response.setHeader('Content-Type', 'text/html');
          response.end(harness);
        });
      },
      transform(source, id) {
        if (id.split('?')[0].replaceAll('\\', '/').endsWith('/services/matting/matting.worker.ts')) {
          return { code: `${noCacheStorage}\n${noWebCrypto}\n${source}`, map: null };
        }
      },
    }],
    server: { port: 0, strictPort: false, host: '127.0.0.1',
      proxy: { '/api': { target: `http://127.0.0.1:${backendPort}`, changeOrigin: false } } },
  });
  await vite.listen();
  const address = vite.httpServer.address();
  assert(address && typeof address !== 'string');
  return `http://127.0.0.1:${address.port}`;
}

try {
  assert(process.env.NOMIFUN_MATTING_MODEL_PATH, 'Set NOMIFUN_MATTING_MODEL_PATH to the approved model');
  const model = await readFile(resolve(process.env.NOMIFUN_MATTING_MODEL_PATH));
  assert.equal(model.length, manifest.size, 'Model size differs from the approved manifest');
  assert.equal(createHash('sha256').update(model).digest('hex'), manifest.sha256, 'Model digest differs from the approved manifest');
  const { chromium } = await loadPlaywright();
  stage = 'FIXTURE';
  const fixture = await startBackend(model);
  const rendererOrigin = await startRenderer(fixture.port);
  fixture.setRendererOrigin(rendererOrigin);
  stage = 'BROWSER';
  browser = await chromium.launch({ headless: true,
    ...(process.env.NOMIFUN_MATTING_BROWSER_EXECUTABLE
      ? { executablePath: process.env.NOMIFUN_MATTING_BROWSER_EXECUTABLE }
      : { channel: process.env.NOMIFUN_MATTING_BROWSER_CHANNEL ?? 'msedge' }) });

  for (const mode of ['desktop', 'webui']) {
    stage = `${mode.toUpperCase()}_MODEL`;
    console.log(`NOMIFUN_MATTING_BROWSER stage=${stage}`);
    const context = await browser.newContext({ viewport: { width: 1280, height: 800 } });
    try {
      // Deliberately seed a cookie in both contexts: desktop cross-origin requests
      // must omit it even though a host cookie also matches the backend port.
      await context.addCookies([{ name: 'matting_fixture_session', value: fixture.cookie, url: rendererOrigin, sameSite: 'Lax' }]);
      await context.addInitScript(({ mode, port, secret }) => {
        Object.defineProperty(globalThis, 'caches', { configurable: true, get() { throw new Error('Cache Storage disabled by matting browser regression'); } });
        if (mode === 'desktop') {
          window.__backendPort = port;
          window.__nomiLocalTrust = secret;
        } else if (globalThis.crypto) {
          Object.defineProperty(globalThis.crypto, 'subtle', { configurable: true,
            get() { throw new Error('Web Crypto disabled by matting browser regression'); } });
        }
      }, { mode, port: fixture.port, secret: fixture.secret });
      const page = await context.newPage();
      const pageErrors = [];
      const workers = [];
      let wasmResponses = 0;
      page.on('pageerror', error => pageErrors.push(error.message));
      page.on('worker', worker => workers.push(worker.url()));
      page.on('response', response => {
        if (/ort-wasm.*\.wasm(?:\?|$)/.test(response.url()) && response.ok()) wasmResponses++;
      });
      await page.goto(`${rendererOrigin}${harnessPath}`, { waitUntil: 'domcontentloaded' });
      await page.waitForFunction(() => Boolean(window.__mattingBrowser), undefined, { timeout: 30000 });
      const cacheDisabled = await page.evaluate(() => { try { return !globalThis.caches; } catch { return true; } });
      assert(cacheDisabled, 'Cache Storage test precondition was not installed');
      const webCryptoDisabled = await page.evaluate(() => { try { return !globalThis.crypto?.subtle; } catch { return true; } });
      assert.equal(webCryptoDisabled, mode === 'webui', 'Web Crypto test precondition was not installed');

      if (mode === 'desktop') {
        const oldRequestFailed = await page.evaluate(async ({ port, secret, sha256 }) => {
          try {
            await fetch(`http://127.0.0.1:${port}/api/companion/matting-model?sha256=${sha256}`, {
              headers: { 'x-nomi-local-trust': secret }, credentials: 'include',
            });
            return false;
          } catch { return true; }
        }, { port: fixture.port, secret: fixture.secret, sha256: manifest.sha256 });
        assert(oldRequestFailed, 'Fixture unexpectedly permits the retired credentialed-CORS request');
      }

      const requestStart = requestLog.length;
      const acquired = await page.evaluate(verifyDigest => window.__mattingBrowser.acquire(verifyDigest), mode === 'desktop');
      assert.equal(acquired.size, manifest.size);
      if (mode === 'desktop') assert.equal(acquired.sha256, manifest.sha256);
      const transfers = requestLog.slice(requestStart).filter(request => request.method === 'GET');
      assert.equal(transfers.length, 1, 'Model was not downloaded exactly once');
      assert.equal(transfers[0].localTrust, mode === 'desktop');
      assert.equal(transfers[0].sessionCookie, mode === 'webui');
      stage = `${mode.toUpperCase()}_INFERENCE`;
      console.log(`NOMIFUN_MATTING_BROWSER stage=${stage}`);
      const inference = await page.evaluate(() => window.__mattingBrowser.infer());
      assert(inference.messages.some(message => message.type === 'progress' && message.phase === 'infer'), 'ONNX session never reached inference');
      assert(!inference.messages.some(message => message.type === 'fallback' && message.reason !== 'unsuitable_image'), JSON.stringify(inference.messages));
      assert.equal(inference.messages.at(-1)?.type, 'done', JSON.stringify(inference.messages));
      assert(workers.some(url => url.includes('matting.worker.ts')), 'The real matting worker was not created');
      assert.equal(requestLog.slice(requestStart).filter(request => request.method === 'GET').length, 1, 'Worker or client fetched another model');
      assert.deepEqual(pageErrors, [], 'Unexpected browser script exception');
      report.modes.push({ mode, cacheStorageDisabled: true, webCryptoDisabled, authenticatedBy: mode === 'desktop' ? 'local-trust' : 'same-origin-cookie',
        modelBytes: acquired.size, modelTransfers: transfers.length, workerCreated: true, wasmResponses, ...inference });
    } finally { await context.close(); }
  }
  if (process.env.NOMIFUN_MATTING_BROWSER_REPORT) {
    await writeFile(resolve(process.env.NOMIFUN_MATTING_BROWSER_REPORT), `${JSON.stringify(report, null, 2)}\n`);
  }
  console.log(`NOMIFUN_MATTING_BROWSER status=pass ${JSON.stringify(report)}`);
} catch (error) {
  console.error(`NOMIFUN_MATTING_BROWSER status=fail stage=${stage} detail=${error instanceof Error ? error.message : String(error)}`);
  process.exitCode = 1;
} finally {
  await browser?.close().catch(() => {});
  await vite?.close().catch(() => {});
  if (backend) await new Promise(accept => backend.close(accept));
}
