/** Real shared-renderer navigation against one disposable live-provider fixture.
 * No model key, provider text, screenshots, or browser console logs are retained.
 * The desktop host's backend-port injection is reproduced in a fresh Edge context.
 */
import { resolve, dirname } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
let browser;
let vite;
let stage = 'BOOT';
let passed = false;
const probe = process.argv.includes('--probe');
const port = Number(process.argv[2]);
const session = process.argv[3];
const control = `http://127.0.0.1:${port}/__reasoning-smoke/ui-state`;
const setStage = async (value) => {
  const response = await fetch(control, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ stage: value }) });
  if (response.status !== 204) throw new Error('control failed');
};
const startRenderer = async () => {
  const { createServer } = await import(pathToFileURL(resolve(root, 'ui/node_modules/vite/dist/node/index.js')).href);
  vite = await createServer({ configFile: resolve(root, 'ui/vite.config.ts'), mode: 'development', logLevel: 'silent',
    server: { port: 0, strictPort: false, host: '127.0.0.1' } });
  await vite.listen();
  const address = vite.httpServer.address();
  if (!address || typeof address === 'string') throw new Error('vite unavailable');
  return `http://127.0.0.1:${address.port}/`;
};
const readPhases = async () => {
  const response = await fetch(`http://127.0.0.1:${port}/api/agent-sessions/${session}/message-history?page_size=500`);
  const page = await response.json();
  if (!response.ok || !Array.isArray(page.data?.items)) throw new Error('history unavailable');
  return page.data.items.filter(item => item.type === 'thinking').map(item => {
    if (!/^[a-f0-9-]{36}$/.test(item.msg_id ?? '') || !['thinking', 'done'].includes(item.content?.status)) throw new Error('history phase invalid');
    return { id: item.msg_id, status: item.content.status };
  }).sort((a, b) => a.id.localeCompare(b.id));
};
const assertReturnedPhases = async (page) => {
  // Two cold reads bracket the DOM check. A concurrent phase transition must
  // be sampled again rather than comparing two different canonical moments.
  for (let attempt = 0; attempt < 10; attempt++) {
    const before = await readPhases();
    if (before.length === 0) throw new Error('thinking history missing');
    let matches = true;
    try {
      await page.waitForFunction((rows) => rows.every(row => {
        const phase = document.querySelector(`[data-thinking-process-identity="${row.id}"]`);
        const running = row.status === 'thinking';
        return phase?.getAttribute('data-thinking-process-state') === (running ? 'running' : 'completed')
          && phase.querySelector('[data-thinking-process-header]')?.getAttribute('aria-expanded') === String(running);
      }), before, { timeout: 500 });
    } catch { matches = false; }
    const after = await readPhases();
    if (JSON.stringify(before) !== JSON.stringify(after)) continue;
    if (!matches) throw new Error('canonical phase disagrees with returned renderer');
    return { thinking: before.filter(row => row.status === 'thinking').length,
      done: before.filter(row => row.status === 'done').length };
  }
  throw new Error('phase remained unstable');
};
try {
  const playwrightPath = process.env.NOMIFUN_LIVE_PLAYWRIGHT_MODULE;
  if (!playwrightPath || (!probe && (!Number.isInteger(port) || port < 1 || port > 65535 || !/^[a-f0-9-]{36}$/.test(session ?? '')))) throw new Error('inputs invalid');
  const { chromium } = await import(pathToFileURL(resolve(playwrightPath)).href);
  browser = await chromium.launch({ channel: 'msedge', headless: true });
  if (probe) {
    stage = 'VITE_PROBE';
    if (!(await fetch(await startRenderer())).ok) throw new Error('renderer unavailable');
    console.log('NOMIFUN_LIVE_REASONING_BROWSER_PROBE status=pass');
    passed = true;
  } else {
    stage = 'SETTINGS';
    const settings = await fetch(`http://127.0.0.1:${port}/api/settings/client`, {
      method: 'PUT', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ language: 'en-US', 'chat.thinking.visible': true, 'chat.thinking.contentLength': 'compact' }),
    });
    if (!settings.ok) throw new Error('settings rejected');
    const renderer = await startRenderer();
    const context = await browser.newContext({ viewport: { width: 1280, height: 800 } });
    await context.addInitScript((backendPort) => { window.__backendPort = backendPort; }, port);
    const page = await context.newPage();
    page.setDefaultTimeout(30_000);
    const protocol = await context.newCDPSession(page);
    await protocol.send('Network.enable');
    const fixtureSockets = new Set();
    let connected = false;
    let notifyConnected;
    protocol.on('Network.webSocketCreated', (event) => {
      if (event.url === `ws://127.0.0.1:${port}/ws`) fixtureSockets.add(event.requestId);
    });
    protocol.on('Network.webSocketHandshakeResponseReceived', (event) => {
      if (fixtureSockets.has(event.requestId) && event.response.status === 101) {
        connected = true;
        notifyConnected?.();
      }
    });
    stage = 'MOUNT_GUID';
    // Load both real routes before the short task starts; navigation timing
    // should measure remount recovery rather than first-time module compilation.
    await page.goto(`${renderer}#/guid`, { waitUntil: 'domcontentloaded' });
    await page.getByTestId('guid-input').waitFor();
    stage = 'MOUNT';
    await page.evaluate((id) => { location.hash = `/conversation/${id}`; }, session);
    await page.locator('[data-conversation-layout]').waitFor();
    stage = 'WS_READY';
    if (!connected) await new Promise((accept, reject) => {
      const timeout = setTimeout(() => reject(new Error('websocket handshake missing')), 30_000);
      notifyConnected = () => { clearTimeout(timeout); accept(); };
    });
    await setStage('ready');
    stage = 'FIRST_THINKING';
    const runningThought = page.locator('[data-thinking-process-state="running"]').first();
    await runningThought.waitFor({ timeout: 90_000 });
    if (await runningThought.getAttribute('data-thinking-body-length') !== 'compact') throw new Error('compact preference not applied');
    if (await runningThought.locator('[data-thinking-process-header]').getAttribute('aria-expanded') !== 'true') throw new Error('active thought closed');
    const live = page.locator('.turn-process-disclosure--live').first();
    if (await live.locator('.turn-process-disclosure__toggle').getAttribute('aria-expanded') !== 'true') throw new Error('active turn closed');

    stage = 'LEAVE';
    await page.evaluate(() => { location.hash = '/guid'; });
    await page.locator('[data-conversation-layout]').waitFor({ state: 'detached' });
    stage = 'RETURN';
    await page.evaluate((id) => { location.hash = `/conversation/${id}`; }, session);
    await page.locator('[data-conversation-layout]').waitFor();
    await live.waitFor();
    await page.waitForFunction(() => document.querySelector('.turn-process-disclosure--live .turn-process-disclosure__toggle')?.getAttribute('aria-expanded') === 'true');
    stage = 'RETURN_PHASE';
    const returned = await assertReturnedPhases(page);
    stage = 'CLOCK';
    const before = await live.locator('.turn-process-disclosure__label').textContent();
    if (!before || before.includes('--')) throw new Error('work clock missing');
    await page.waitForFunction((previous) => {
      const current = document.querySelector('.turn-process-disclosure--live .turn-process-disclosure__label')?.textContent;
      return Boolean(current && current !== previous);
    }, before, { timeout: 2500 });
    stage = 'PHASE_DONE';
    const completed = page.locator('.turn-process-disclosure--live [data-thinking-process-state="completed"]').first();
    await completed.waitFor();
    await page.waitForFunction(() => {
      const headers = [...document.querySelectorAll('.turn-process-disclosure--live [data-thinking-process-state="completed"] [data-thinking-process-header]')];
      return headers.length > 0 && headers.every((header) => header.getAttribute('aria-expanded') === 'false');
    });
    stage = 'TERMINAL';
    await live.waitFor({ state: 'detached', timeout: 150_000 });
    // A second cold mount after completion must not revive the live work clock.
    await page.evaluate(() => { location.hash = '/guid'; });
    await page.locator('[data-conversation-layout]').waitFor({ state: 'detached' });
    await page.evaluate((id) => { location.hash = `/conversation/${id}`; }, session);
    await page.locator('[data-conversation-layout]').waitFor();
    await page.locator('.turn-process-disclosure').first().waitFor();
    if (await page.locator('.turn-process-disclosure--live').count() !== 0) throw new Error('terminal revived');
    const outer = page.locator('.turn-process-disclosure__toggle[aria-expanded]').first();
    if (await outer.getAttribute('aria-expanded') === 'false') await outer.click();
    const doneHeaders = page.locator('[data-thinking-process-state="completed"] [data-thinking-process-header]');
    if (await doneHeaders.count() === 0) throw new Error('completed history missing');
    for (const header of await doneHeaders.all()) if (await header.getAttribute('aria-expanded') !== 'false') throw new Error('completed history expanded');
    await setStage('passed');
    passed = true;
    console.log(`NOMIFUN_LIVE_REASONING_NAVIGATION leaves=2 returns=2 returned_thinking=${returned.thinking} returned_done=${returned.done} clock_advances=true returned_expanded=true completed_collapsed=true terminal=true`);
  }
} catch {
  console.error(`NOMIFUN_LIVE_REASONING_NAVIGATION_FAILURE code=NAVIGATION_${stage}_FAILED`);
  if (!probe) { try { await setStage('failed'); } catch {} }
  process.exitCode = 1;
} finally {
  await browser?.close().catch(() => {});
  await vite?.close().catch(() => {});
  if (!passed) process.exitCode = 1;
}
