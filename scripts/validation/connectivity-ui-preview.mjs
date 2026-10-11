#!/usr/bin/env node
/** Authenticated, isolated preview of the real Desktop renderer and Mobile H5.
 * No API responses, client identities, or device credentials are mocked. */
import assert from 'node:assert/strict';
import { randomBytes } from 'node:crypto';
import { createReadStream, existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, statSync, writeFileSync } from 'node:fs';
import http from 'node:http';
import https from 'node:https';
import { dirname, extname, isAbsolute, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { freePort, Processes, request, until } from './connectivity-stack-helpers.mjs';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const HELP = `Authenticated Desktop + Mobile H5 UI preview (Node 24+).

node scripts/validation/connectivity-ui-preview.mjs [--desktop-bin target/debug/nomifun-web.exe] [--desktop-dist ui/dist] [--mobile-dist ../nomifun-mobile/dist | --mobile-upstream http://127.0.0.1:18781] [--duration-minutes 20] [--remove-fixture]
node scripts/validation/connectivity-ui-preview.mjs --stop <private-preview-control.json>

Build the latest binaries and UI separately before use. This helper never builds,
kills a port owner, or uses production data. All listeners bind to 127.0.0.1 on
allocated ports; port 25811 is untouched. Credentials are saved only in a private
ignored fixture file. Use Ctrl+C or wait for the duration limit to stop owned
servers and the directly spawned Web host. Fixtures are retained by default.
`;

export function parseArguments(args) {
  if (args[0] === '--stop') { assert(args.length === 2, '--stop requires exactly one private control file'); return { stopFile: resolve(args[1]) }; }
  const config = {
    desktopBin: resolve(ROOT, `target/debug/nomifun-web${process.platform === 'win32' ? '.exe' : ''}`),
    desktopDist: resolve(ROOT, 'ui/dist'),
    mobileDist: resolve(ROOT, '../nomifun-mobile/dist'),
    durationMinutes: 20, removeFixture: false,
  };
  const paths = new Map([['--desktop-bin', 'desktopBin'], ['--desktop-dist', 'desktopDist'], ['--mobile-dist', 'mobileDist']]);
  for (let index = 0; index < args.length; index++) {
    const arg = args[index];
    if (arg === '--help') return { help: true };
    if (arg === '--remove-fixture') { config.removeFixture = true; continue; }
    assert(args[index + 1] && !args[index + 1].startsWith('--'), `Missing value: ${arg}`);
    const value = args[++index];
    if (paths.has(arg)) { config[paths.get(arg)] = resolve(ROOT, value); continue; }
    if (arg === '--duration-minutes') { config.durationMinutes = Number(value); continue; }
    if (arg === '--mobile-upstream') { config.mobileUpstream = new URL(value); continue; }
    throw new Error(`Unknown option: ${arg}`);
  }
  assert(existsSync(config.desktopBin), 'The compiled nomifun-web binary must exist; build it separately');
  assert(existsSync(join(config.desktopDist, 'index.html')), 'Desktop ui/dist must contain the latest built renderer');
  assert(Number.isFinite(config.durationMinutes) && config.durationMinutes > 0 && config.durationMinutes <= 120, '--duration-minutes must be between 0 and 120');
  if (config.mobileUpstream) {
    assert(['http:', 'https:'].includes(config.mobileUpstream.protocol), 'Mobile upstream must use HTTP(S)');
    assert(['localhost', '127.0.0.1', '[::1]'].includes(config.mobileUpstream.hostname), 'Mobile upstream must be loopback');
    assert(!config.mobileUpstream.username && !config.mobileUpstream.password && config.mobileUpstream.pathname === '/' && !config.mobileUpstream.search && !config.mobileUpstream.hash, 'Mobile upstream must be a credential-free origin');
  } else assert(existsSync(join(config.mobileDist, 'index.html')), 'Provide a current Mobile export or --mobile-upstream');
  return config;
}

const API_PATH = /^\/(?:api(?:\/|$)|login(?:\/|$)|logout(?:\/|$)|qr-login(?:\/|$)|health(?:\/|$)|ws(?:\/|$))/;
const MIME = { '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.mjs': 'text/javascript; charset=utf-8', '.css': 'text/css; charset=utf-8', '.json': 'application/json', '.svg': 'image/svg+xml', '.png': 'image/png', '.jpg': 'image/jpeg', '.jpeg': 'image/jpeg', '.webp': 'image/webp', '.ico': 'image/x-icon', '.woff': 'font/woff', '.woff2': 'font/woff2', '.ttf': 'font/ttf', '.wasm': 'application/wasm', '.map': 'application/json' };

function forwardHttp(target, incoming, outgoing) {
  const transport = target.protocol === 'https:' ? https : http;
  // Preserve the frontend Host and Origin, matching Vite's changeOrigin:false.
  // Cookie authentication and CSRF remain ordinary product behavior.
  const headers = { ...incoming.headers };
  delete headers.connection;
  const upstream = transport.request(new URL(incoming.url, target), { method: incoming.method, headers, rejectUnauthorized: true }, response => {
    outgoing.writeHead(response.statusCode, response.headers);
    response.pipe(outgoing);
  });
  upstream.setTimeout(60_000, () => upstream.destroy(new Error('Upstream timeout')));
  upstream.once('error', () => { if (!outgoing.headersSent) outgoing.writeHead(502); outgoing.end('preview upstream unavailable'); });
  incoming.once('aborted', () => upstream.destroy());
  outgoing.once('close', () => { if (!outgoing.writableFinished) upstream.destroy(); });
  incoming.pipe(upstream);
}

async function frontend({ api, dist, upstream, expo = false }) {
  const apiTarget = new URL(api);
  const directory = dist ? realpathSync(dist) : null;
  const sockets = new Set();
  const server = http.createServer((incoming, outgoing) => {
    const pathname = new URL(incoming.url, 'http://127.0.0.1').pathname;
    if (API_PATH.test(pathname)) { forwardHttp(apiTarget, incoming, outgoing); return; }
    if (upstream) { forwardHttp(upstream, incoming, outgoing); return; }
    if (!['GET', 'HEAD'].includes(incoming.method)) { outgoing.writeHead(405).end(); return; }
    let decoded;
    try { decoded = decodeURIComponent(pathname); } catch { outgoing.writeHead(400).end(); return; }
    const candidate = resolve(directory, `.${decoded}`);
    const within = relative(directory, candidate);
    if (within.startsWith(`..${sep}`) || within === '..' || isAbsolute(within)) { outgoing.writeHead(403).end(); return; }
    const possible = expo ? [candidate, `${candidate}.html`, join(candidate, 'index.html')] : [candidate];
    let file = possible.find(value => existsSync(value) && statSync(value).isFile());
    if (!file && !extname(decoded)) file = join(directory, 'index.html');
    if (!file || !existsSync(file)) { outgoing.writeHead(404).end(); return; }
    const real = realpathSync(file), realWithin = relative(directory, real);
    if (realWithin.startsWith(`..${sep}`) || realWithin === '..' || isAbsolute(realWithin)) { outgoing.writeHead(403).end(); return; }
    outgoing.writeHead(200, { 'content-type': MIME[extname(file)] ?? 'application/octet-stream', 'cache-control': 'no-store', 'content-length': statSync(file).size });
    if (incoming.method === 'HEAD') outgoing.end(); else createReadStream(file).on('error', () => outgoing.destroy()).pipe(outgoing);
  });
  server.on('connection', socket => { sockets.add(socket); socket.once('close', () => sockets.delete(socket)); });
  server.on('upgrade', (incoming, downstream, head) => {
    const target = API_PATH.test(new URL(incoming.url, 'http://127.0.0.1').pathname) ? apiTarget : upstream;
    if (!target) { downstream.end('HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\n'); return; }
    const transport = target.protocol === 'https:' ? https : http;
    const forward = transport.request(new URL(incoming.url, target), { headers: incoming.headers, rejectUnauthorized: true });
    forward.once('upgrade', (response, remote, remoteHead) => {
      sockets.add(remote); remote.once('close', () => sockets.delete(remote));
      downstream.write(`HTTP/1.1 101 Switching Protocols\r\n${Object.entries(response.headers).map(([name, value]) => `${name}: ${value}\r\n`).join('')}\r\n`);
      if (head.length) remote.write(head);
      if (remoteHead.length) downstream.write(remoteHead);
      downstream.pipe(remote).pipe(downstream);
      downstream.once('close', () => remote.destroy()); remote.once('close', () => downstream.destroy());
      downstream.on('error', () => remote.destroy()); remote.on('error', () => downstream.destroy());
    });
    forward.once('response', response => { response.resume(); downstream.end(`HTTP/1.1 ${response.statusCode} Rejected\r\nConnection: close\r\n\r\n`); });
    forward.once('error', () => downstream.destroy());
    forward.end();
  });
  await new Promise((accept, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', accept); });
  assert.notEqual(server.address().port, 25811, 'Reserved native port cannot be used for a preview frontend');
  return {
    url: `http://127.0.0.1:${server.address().port}`,
    close: async () => { for (const socket of sockets) socket.destroy(); await new Promise(accept => server.close(accept)); },
  };
}

async function controller(token, stop) {
  const sockets = new Set();
  const server = http.createServer((incoming, outgoing) => {
    if (incoming.method !== 'POST' || incoming.url !== '/__preview/stop' || incoming.headers['x-preview-control'] !== token) { outgoing.writeHead(403).end(); return; }
    outgoing.writeHead(200, { 'content-type': 'application/json' }).end('{"stopped":true}');
    setImmediate(stop);
  });
  server.on('connection', socket => { sockets.add(socket); socket.once('close', () => sockets.delete(socket)); });
  await new Promise((accept, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', accept); });
  return { url: `http://127.0.0.1:${server.address().port}/__preview/stop`, close: async () => { for (const socket of sockets) socket.destroy(); await new Promise(accept => server.close(accept)); } };
}

async function stopPreview(file) {
  const control = JSON.parse(readFileSync(file, 'utf8'));
  const target = new URL(control.url);
  assert(target.protocol === 'http:' && target.hostname === '127.0.0.1' && target.pathname === '/__preview/stop' && !target.search && !target.username && !target.password, 'Invalid private preview controller');
  assert(/^[a-f0-9]{64}$/.test(control.token), 'Invalid private controller token');
  assert.equal((await request(target, { method: 'POST', headers: { 'x-preview-control': control.token } })).status, 200, 'Owned preview controller rejected stop');
  process.stdout.write('Requested stop of the owned UI preview.\n');
}

export async function run(config) {
  assert(!process.versions.bun && Number(process.versions.node.split('.')[0]) >= 24, 'Use Node 24+');
  const parent = resolve(ROOT, '.tmp-connectivity-ui-preview');
  mkdirSync(parent, { recursive: true });
  const parentReal = realpathSync(parent);
  const fixture = mkdtempSync(join(parentReal, 'run-'));
  const marker = randomBytes(24).toString('hex');
  writeFileSync(join(fixture, '.preview-owned'), marker, { mode: 0o600 });
  const data = join(fixture, 'data');
  mkdirSync(data);
  const credentials = { username: 'preview-owner', password: `Preview-${randomBytes(24).toString('base64url')}!` };
  const credentialFile = join(fixture, 'owner-credentials.json');
  writeFileSync(credentialFile, JSON.stringify(credentials, null, 2), { mode: 0o600 });
  const processes = new Processes(fixture);
  processes.secrets.add(credentials.password);
  const resources = [];
  let timer;
  let exiting = false;
  let resolveHold;
  const hold = new Promise(accept => { resolveHold = accept; });
  const stop = () => resolveHold();
  process.once('SIGINT', stop);
  process.once('SIGTERM', stop);
  let web;
  let stage = 'web-startup';
  try {
    const apiPort = await freePort();
    assert.notEqual(apiPort, 25811, 'Reserved native port cannot be used for the Web host');
    web = processes.start('web', config.desktopBin, ['--host', '127.0.0.1', '--port', String(apiPort), '--data-dir', data, '--api-only'], {
      NOMIFUN_WEB_INSECURE_NO_AUTH: 'false', NOMIFUN_WEB_HOST: '127.0.0.1', NOMIFUN_WEB_PORT: String(apiPort),
      NOMIFUN_DATA_DIR: data, NOMIFUN_WORK_DIR: join(data, 'work'), NOMIFUN_CACHE_DIR: join(data, 'cache'), NOMIFUN_LOG_DIR: join(data, 'logs'),
      NOMIFUN_ADMIN_USERNAME: credentials.username, NOMIFUN_ADMIN_PASSWORD: credentials.password,
      NOMIFUN_HTTPS: 'false', NO_PROXY: '127.0.0.1,localhost', no_proxy: '127.0.0.1,localhost',
    });
    const api = await until('isolated authenticated Web host', async () => {
      assert(web.child.exitCode === null, 'Owned Web host exited before readiness');
      const announced = join(data, 'port.json');
      if (!existsSync(announced)) return false;
      const port = JSON.parse(readFileSync(announced, 'utf8')).port;
      if (!Number.isInteger(port) || port < 1 || port === 25811) return false;
      const url = `http://127.0.0.1:${port}`;
      const response = await request(`${url}/api/auth/status`);
      if (response.status !== 200) return false;
      const anonymous = await request(`${url}/api/auth/user`);
      assert([401, 403].includes(anonymous.status), 'Preview must require ordinary authentication');
      return url;
    }, { timeoutMs: 60_000 });
    stage = 'mobile-upstream';
    if (config.mobileUpstream) {
      const upstreamResponse = await request(config.mobileUpstream, { timeoutMs: 60_000 });
      assert.equal(upstreamResponse.status, 200, `Mobile upstream returned HTTP ${upstreamResponse.status}`);
    }
    stage = 'frontend-startup';
    const desktop = await frontend({ api, dist: config.desktopDist }); resources.push(desktop);
    const mobile = await frontend({ api, dist: config.mobileDist, upstream: config.mobileUpstream, expo: true }); resources.push(mobile);
    for (const surface of [desktop, mobile]) {
      assert.equal((await request(`${surface.url}/api/auth/status`)).status, 200, 'Frontend API proxy must be reachable');
      const document = await request(surface.url, { timeoutMs: 60_000 });
      assert.equal(document.status, 200, `Frontend document returned HTTP ${document.status}`);
    }
    stage = 'owner-login';
    const login = await request(`${desktop.url}/login`, { method: 'POST', headers: { Origin: desktop.url }, body: credentials });
    assert.equal(login.status, 200, 'Ordinary owner login through the frontend proxy must succeed');
    assert(login.json?.success !== false, 'Owner login must succeed');
    const cookies = (Array.isArray(login.headers['set-cookie']) ? login.headers['set-cookie'] : [login.headers['set-cookie']]).filter(Boolean).map(value => value.split(';')[0]).join('; ');
    assert(cookies, 'Owner login must issue ordinary cookies');
    assert.equal((await request(`${desktop.url}/api/connections/devices`, { headers: { Cookie: cookies } })).status, 200, 'Authenticated owner must reach device management through the proxy');
    const controlToken = randomBytes(32).toString('hex');
    processes.secrets.add(controlToken);
    const control = await controller(controlToken, stop); resources.push(control);
    const controlFile = join(fixture, 'preview-control.json');
    writeFileSync(controlFile, JSON.stringify({ url: control.url, token: controlToken }), { mode: 0o600 });
    const manifest = {
      desktop_url: `${desktop.url}/#/connections`, mobile_url: `${mobile.url}/connect`, api_url: api,
      credential_file: credentialFile, control_file: controlFile, fixture_dir: fixture, duration_minutes: config.durationMinutes,
      mobile_source: config.mobileUpstream ? config.mobileUpstream.href : config.mobileDist,
      desktop_source: config.desktopDist, evidence: 'real authenticated API and real frontend builds; loopback only',
      native_lifecycle: false, phone_hardware: false,
    };
    writeFileSync(join(fixture, 'preview.json'), JSON.stringify(manifest, null, 2));
    process.stdout.write(`${JSON.stringify(manifest, null, 2)}\n`);
    timer = setTimeout(stop, config.durationMinutes * 60_000);
    web.child.once('exit', () => { if (!exiting) stop(); });
    await hold;
  } catch (error) {
    error.previewStage = stage;
    error.previewDiagnostic = processes.redact(error.message);
    error.previewLog = join(fixture, 'web.log');
    throw error;
  } finally {
    exiting = true;
    clearTimeout(timer);
    process.removeListener('SIGINT', stop); process.removeListener('SIGTERM', stop);
    const cleanupErrors = [];
    for (const resource of resources.reverse()) try { await resource.close(); } catch (error) { cleanupErrors.push(error); }
    if (web) processes.log(web);
    try { await processes.close(); } catch (error) { cleanupErrors.push(error); }
    if (config.removeFixture && cleanupErrors.length === 0) {
      const actual = realpathSync(fixture), within = relative(parentReal, actual);
      assert(within && !within.startsWith(`..${sep}`) && within !== '..' && !isAbsolute(within), 'Cleanup escaped the owned preview parent');
      assert.equal(readFileSync(join(actual, '.preview-owned'), 'utf8'), marker, 'Preview ownership marker changed');
      rmSync(actual, { recursive: true, force: true });
    }
    process.stdout.write('Preview stopped; owned servers and Web host closed.\n');
    if (cleanupErrors.length) throw new AggregateError(cleanupErrors, 'Owned preview cleanup failed');
  }
}

if (process.argv[1] && realpathSync(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { const config = parseArguments(process.argv.slice(2)); config.help ? process.stdout.write(HELP) : config.stopFile ? await stopPreview(config.stopFile) : await run(config); }
  catch (error) {
    process.stderr.write(`UI preview failed at ${error.previewStage ?? 'arguments'}: ${error.previewDiagnostic ?? error.code ?? error.name ?? 'Error'}\n`);
    if (error.previewLog) process.stderr.write(`Private redacted Web host log: ${error.previewLog}\n`);
    process.exitCode = 1;
  }
}
