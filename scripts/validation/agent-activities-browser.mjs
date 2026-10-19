/** Real desktop WebUI user-control smoke against an opt-in disposable Rust fixture.
 * Uses original AppServices, canonical input facts and HTTP routes. No model or
 * IPC responses are mocked. It never opens the user's browser profile/dataset.
 */
import { mkdtemp, readFile, writeFile, realpath, stat } from 'node:fs/promises';
import { createReadStream } from 'node:fs';
import { tmpdir } from 'node:os';
import { resolve, dirname, join, isAbsolute } from 'node:path';
import { pathToFileURL, fileURLToPath } from 'node:url';
import { randomUUID, createHash } from 'node:crypto';
import { spawn } from 'node:child_process';
import { createServer as createHttpServer } from 'node:http';

const repository = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const option = (name) => { const at = process.argv.indexOf(name); return at < 0 ? undefined : process.argv[at + 1]; };
const binary = option('--backend-test');
const playwrightModule = option('--playwright-module') ?? process.env.NOMIFUN_LIVE_PLAYWRIGHT_MODULE;
if (!binary || !playwrightModule || !isAbsolute(binary) || !isAbsolute(playwrightModule)) {
  throw new Error('Pass absolute --backend-test <compiled nomifun-app lib test exe> and --playwright-module <existing playwright index.mjs>.');
}
const backendPath = await realpath(binary);
const backendFile = await stat(backendPath);
if (!isAbsolute(backendPath) || !backendFile.isFile()) throw new Error('Backend test binary must resolve to an existing absolute file.');
const backendHash = createHash('sha256');
for await (const chunk of createReadStream(backendPath)) backendHash.update(chunk);
const backendIdentity = { path: backendPath, sha256: backendHash.digest('hex'), modified_at_utc: backendFile.mtime.toISOString(), length_bytes: backendFile.size };
const root = await mkdtemp(join(tmpdir(), 'nomifun-activity-browser-'));
const nonce = randomUUID();
await writeFile(join(root, '.activity-browser-fixture.json'), JSON.stringify({ kind: 'nomifun.agent-activity-browser.v1', nonce }), { flag: 'wx' });
let child, exited, browser, context, vite, rendererServer, backend, page;
let stage = 'backend-start';
let passed = false;
const logs = [];
const report = { root, backend_test_binary: backendIdentity, viewports: [], checks: [] };
const rendererErrors = [];
const browserMessages = [];
const requestFailures = [];
const captureRenderer = async (label) => {
  if (!page || page.isClosed()) return;
  await page.screenshot({ path: join(root, `${label}.png`), fullPage: true }).catch(() => {});
  await writeFile(join(root, `${label}.html`), await page.content()).catch(() => {});
  report.renderer = { url: page.url(), errors: rendererErrors, messages: browserMessages, requestFailures,
    ...(await page.evaluate(() => ({ title: document.title, language: document.documentElement.lang,
      body: document.body?.innerText.slice(0, 12_000),
      controls: [...document.querySelectorAll('button,a,[role="button"],[role="tab"]')].map((element) => ({
        role: element.getAttribute('role') ?? element.tagName.toLowerCase(), label: element.getAttribute('aria-label'), text: element.textContent?.slice(0, 200) })) })).catch(() => ({}))) };
};
const delay = (ms) => new Promise((accept) => setTimeout(accept, ms));
const bounded = async (promise, ms) => {
  let timer;
  try { return await Promise.race([promise, new Promise((accept) => { timer = setTimeout(() => accept(undefined), ms); })]); }
  finally { clearTimeout(timer); }
};
const until = async (test, description, timeout = 40_000) => {
  const end = Date.now() + timeout;
  while (Date.now() < end) { const result = await test(); if (result) return result; await delay(100); }
  throw new Error(`Timed out: ${description}`);
};
const fixture = async (path, action) => {
  const response = await fetch(`${backend}/__agent-activities-fixture/${path}`, {
    method: action ? 'POST' : 'GET', headers: { 'x-nomifun-activity-fixture': nonce, ...(action ? { 'Content-Type': 'application/json' } : {}) },
    ...(action ? { body: JSON.stringify({ action }) } : {}), signal: AbortSignal.timeout(10_000),
  });
  if (!response.ok) throw new Error(`Fixture control ${path}: ${response.status}`);
  return response.json();
};
const verifyNoWork = (status) => {
  if (status.turns !== 0 || status.model_requests !== 0 || status.alpha.claimed || status.beta.claimed) throw new Error('Fixture unexpectedly admitted a Turn or model request');
};
const layout = async (page, viewport) => {
  await page.setViewportSize(viewport);
  const geometry = await page.evaluate(() => {
    const region = document.querySelector('section[aria-label="Active work"]');
    const boxes = [...(region?.querySelectorAll('article, article > div, article .grid') ?? [])];
    return { document: { width: document.documentElement.clientWidth, scroll: document.documentElement.scrollWidth },
      boxes: boxes.map((element) => { const r = element.getBoundingClientRect(); return { width: element.clientWidth, scroll: element.scrollWidth, left: r.left, right: r.right }; }) };
  });
  if (geometry.document.scroll > geometry.document.width + 1 || geometry.boxes.some((box) => box.scroll > box.width + 1 || box.left < -1 || box.right > viewport.width + 1)) {
    throw new Error(`Desktop layout overflow at ${viewport.width}x${viewport.height}`);
  }
  report.viewports.push({ ...viewport, geometry });
  await page.screenshot({ path: join(root, `active-work-${viewport.width}x${viewport.height}.png`), fullPage: true });
};

try {
  child = spawn(backendPath, ['--exact', 'router::nomi_core_session::agent_activities::tests::activity_browser_fixture', '--ignored', '--nocapture', '--test-threads=1'], {
    cwd: repository, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'], env: { ...process.env,
      // Match the repository's existing Cargo test-thread stack setting when
      // invoking this already-compiled test executable directly.
      RUST_MIN_STACK: process.env.RUST_MIN_STACK ?? '134217728',
      NOMIFUN_ACTIVITY_BROWSER_FIXTURE_DIR: root, NOMIFUN_DATA_DIR: join(root, 'data'), NOMIFUN_WORK_DIR: join(root, 'work'),
      NOMIFUN_CACHE_DIR: join(root, 'cache'), NOMIFUN_LOG_DIR: join(root, 'logs'), NOMIFUN_HTTPS: 'false' },
  });
  exited = new Promise((accept, reject) => { child.once('error', reject); child.once('exit', (code, signal) => accept({ code, signal })); });
  let earlyExit;
  let earlyFailure;
  void exited.then((result) => { earlyExit = result; }, (error) => { earlyFailure = error; });
  const retain = (data) => { if (logs.reduce((count, text) => count + text.length, 0) < 1_000_000) logs.push(data.toString()); };
  child.stdout.on('data', retain); child.stderr.on('data', retain);
  const ready = await until(async () => {
    if (earlyFailure) throw earlyFailure;
    if (earlyExit) throw new Error(`Fixture exited before readiness: ${JSON.stringify(earlyExit)}`);
    try { return JSON.parse(await readFile(join(root, 'ready.json'), 'utf8')); } catch { return undefined; }
  }, 'fixture readiness', 90_000);
  if (ready.nonce !== nonce || ready.pid !== child.pid || !Number.isInteger(ready.port) || ready.port < 1 || ready.port > 65535) throw new Error('Fixture readiness identity mismatch');
  backend = `http://127.0.0.1:${ready.port}`;
  verifyNoWork(await fixture('status'));
  stage = 'renderer-start';
  const { createServer } = await import(pathToFileURL(join(repository, 'ui/node_modules/vite/dist/node/index.js')).href);
  const proxy = { target: backend, changeOrigin: false };
  // Vite's regular listen treats 0 as its default port. Its supported
  // middleware host lets the OS choose a genuinely private ephemeral port,
  // while preserving Vite's real SPA, HMR, API and WebSocket proxy middleware.
  rendererServer = createHttpServer((request, response) => vite.middlewares(request, response));
  vite = await createServer({ configFile: join(repository, 'ui/vite.config.ts'), mode: 'webdev', logLevel: 'silent',
    server: { host: '127.0.0.1', middlewareMode: { server: rendererServer }, hmr: { server: rendererServer },
      proxy: { '/api': proxy, '/health': proxy, '/login': proxy, '/logout': proxy, '/ws': { ...proxy, ws: true } } } });
  await new Promise((accept, reject) => { rendererServer.once('error', reject); rendererServer.listen(0, '127.0.0.1', accept); });
  const address = rendererServer.address();
  if (!address || typeof address === 'string') throw new Error('Vite did not bind a private port');
  const { chromium } = await import(pathToFileURL(playwrightModule).href);
  browser = await chromium.launch({ channel: 'msedge', headless: true });
  context = await browser.newContext({ locale: 'en-US', viewport: { width: 1280, height: 800 }, extraHTTPHeaders: { 'x-nomi-local-trust': 'activity-lifecycle-regression' } });
  stage = 'fixture-authentication';
  const renderer = `http://127.0.0.1:${address.port}`;
  // Bootstrap only this brand-new fixture's existing installation owner via
  // the official setup route. Its HttpOnly session cookie authenticates the
  // real WebUI WebSocket; custom HTTP headers cannot authenticate browser WS.
  const setup = await context.request.post(`${renderer}/api/auth/setup`, { data: { username: 'activity-fixture', password: `Fixture-only-${nonce}!` } });
  if (!setup.ok() || (await setup.json()).success !== true) throw new Error(`Fixture owner setup failed (${setup.status()})`);
  if (!(await context.cookies(renderer)).some((cookie) => cookie.name === 'nomifun-session' && cookie.httpOnly)) throw new Error('Official fixture setup did not issue the original HttpOnly session cookie');
  await context.setExtraHTTPHeaders({});
  report.authentication = { transport: 'original-session-cookie', trustedBootstrapOnly: true, locale: 'en-US' };
  page = await context.newPage();
  page.setDefaultTimeout(30_000);
  page.on('pageerror', (error) => rendererErrors.push(error.message));
  page.on('console', (message) => { if (['error', 'warning'].includes(message.type()) && browserMessages.length < 50) browserMessages.push({ type: message.type(), text: message.text().slice(0, 2000) }); });
  page.on('requestfailed', (request) => { if (requestFailures.length < 50) requestFailures.push({ url: request.url(), error: request.failure()?.errorText }); });
  page.on('response', (response) => { if (response.status() >= 400 && requestFailures.length < 50) requestFailures.push({ url: response.url(), status: response.status() }); });
  stage = 'discover';
  await page.goto(`${renderer}/#/guid`, { waitUntil: 'domcontentloaded' });
  await captureRenderer('discover-before-click');
  await page.getByRole('button', { name: 'Work and schedules', exact: true }).click();
  await page.getByRole('tab', { name: 'Active work', exact: true }).waitFor();
  const active = page.getByRole('region', { name: 'Active work' });
  const alpha = active.locator('article').filter({ hasText: 'Lifecycle input Alpha' });
  const beta = active.locator('article').filter({ hasText: 'Lifecycle input Beta' });
  await alpha.getByText('Preparing', { exact: true }).waitFor();
  await beta.getByText('Preparing', { exact: true }).waitFor();
  await layout(page, { width: 1280, height: 800 });
  stage = 'single-stop';
  await alpha.getByRole('button', { name: 'Stop this task', exact: true }).click();
  await until(async () => { const status = await fixture('status'); verifyNoWork(status); return status.alpha.cancelled && status.alpha.signal_requested && status.alpha.preparation_in_flight && !status.beta.cancelled; }, 'single Stop reaches exact held owner');
  await alpha.getByText('Stop accepted', { exact: true }).waitFor();
  await alpha.getByText('Stopping and cleaning up', { exact: true }).waitFor();
  if (await alpha.getByText('Execution stopped', { exact: true }).count()) throw new Error('Stop acceptance falsely reported original owner ended');
  report.checks.push('single exact input Stop persisted and signalled; held owner remains stopping; sibling remains admitted');
  await layout(page, { width: 880, height: 600 });
  stage = 'all-stop';
  await active.getByRole('button', { name: 'Stop all', exact: true }).click();
  await until(async () => { const status = await fixture('status'); verifyNoWork(status); return ['alpha', 'beta'].every((key) => status[key].cancelled && status[key].signal_requested && status[key].preparation_in_flight); }, 'snapshot Stop reaches both original owners');
  await active.getByText('Accepted 2 · Pending 0 · Unconfirmed 0', { exact: true }).waitFor();
  await active.getByText('Execution stop confirmed for 0. Inspect each item\'s resource progress.', { exact: true }).waitFor();
  await beta.getByText('Stop accepted', { exact: true }).waitFor();
  if (await beta.getByText('Execution stopped', { exact: true }).count()) throw new Error('Snapshot falsely reported held owner ended');
  report.checks.push('All Stop captured original work, accepted both independently and reported zero ended owners');
  stage = 'owner-settlement';
  await fixture('command', 'release_alpha');
  await alpha.waitFor({ state: 'detached' });
  await beta.waitFor();
  await fixture('command', 'release_beta');
  await beta.waitFor({ state: 'detached' });
  const final = await fixture('status');
  verifyNoWork(final);
  if (!final.alpha.cancelled || !final.beta.cancelled || final.alpha.preparation_in_flight || final.beta.preparation_in_flight) throw new Error('Original preparation owners did not settle');
  if (rendererErrors.length) throw new Error(`Renderer exception: ${rendererErrors[0]}`);
  report.checks.push('Original owner release removes rows through read-only polling without a Turn, retry or model request');
  report.final = final;
  await page.screenshot({ path: join(root, 'active-work-settled.png'), fullPage: true });
  await context.close(); context = undefined;
  await browser.close(); browser = undefined;
  stage = 'backend-proof';
  await fixture('command', 'finish');
  const result = await bounded(exited, 20_000);
  if (!result) throw new Error('Fixture did not shut down');
  const proof = JSON.parse(await readFile(join(root, 'backend-proof.json'), 'utf8'));
  if (result.code !== 0 || !proof.passed) throw new Error(`Backend ownership proof failed: ${JSON.stringify(result)}`);
  report.backend = proof;
  passed = true;
} catch (error) {
  report.error = String(error);
  await captureRenderer(`failed-${stage}`).catch(() => {});
  process.exitCode = 1;
} finally {
  await context?.close().catch(() => {});
  await browser?.close().catch(() => {});
  await vite?.close().catch(() => {});
  if (rendererServer) {
    rendererServer.closeAllConnections();
    await new Promise((accept) => rendererServer.close(() => accept()));
  }
  if (!passed && backend) {
    for (const action of ['release_alpha', 'release_beta', 'finish']) await fixture('command', action).catch(() => {});
  }
  if (child && !child.killed) {
    const done = await bounded(exited, 15_000).catch(() => undefined);
    // Only this spawned opt-in test process is owned by the driver.
    if (!done) child.kill();
  }
  await writeFile(join(root, 'backend.log'), logs.join(''));
  await writeFile(join(root, 'browser-proof.json'), JSON.stringify({ ...report, passed, stage }, null, 2));
  console.log(`NOMIFUN_ACTIVITY_BROWSER ${passed ? 'PASS' : 'FAIL'} stage=${stage} evidence=${root}`);
  if (!passed) process.exitCode = 1;
}
