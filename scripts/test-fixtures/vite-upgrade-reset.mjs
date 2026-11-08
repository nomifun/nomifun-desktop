// Disposable loopback server only. The unguarded mode intentionally crashes in
// its own child process, keeping the test runner and user's dev server intact.
import { createConnection, createServer as createNetServer } from 'node:net';
import { writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { guardDevUpgradeSockets } from '../lib/vite-upgrade-sockets.mjs';
const requireUi = createRequire(new URL('../../ui/package.json', import.meta.url));
const { createServer, createLogger } = requireUi('vite');

const [mode, root, configuration] = process.argv.slice(2);
const projectConfig = configuration === 'project-config';
if (!['unguarded', 'guarded'].includes(mode) || !root) throw new Error('Pass fixture mode and owned temporary root');
writeFileSync(`${root}/index.html`, '<!doctype html><title>owned vite reset fixture</title>');
const allocation = createNetServer();
await new Promise((resolve, reject) => { allocation.once('error', reject); allocation.listen(0, '127.0.0.1', resolve); });
const port = allocation.address().port;
await new Promise(resolve => allocation.close(resolve));
const logs = [];
const logger = createLogger('silent');
const info = logger.info.bind(logger); const error = logger.error.bind(logger);
logger.info = (message, options) => { logs.push({ level: 'info', message }); info(message, options); };
logger.error = (message, options) => { logs.push({ level: 'error', message, code: options?.error?.code }); error(message, options); };
const server = await createServer({ configFile: projectConfig ? fileURLToPath(new URL('../../ui/vite.config.ts', import.meta.url)) : false,
  root, cacheDir: `${root}/.vite-cache`, customLogger: logger, logLevel: 'silent', clearScreen: false,
  server: { host: '127.0.0.1', port, strictPort: true }, optimizeDeps: { noDiscovery: true, include: [] } });
if (mode === 'guarded' && !projectConfig) guardDevUpgradeSockets(server.httpServer, logger);
await server.listen();
console.log(JSON.stringify({ event: 'listening', port, node: process.version, mode }));
let client;
try {
  const upgraded = new Promise(resolve => server.httpServer.once('upgrade', (request, socket) => {
    const closed = new Promise(done => socket.once('close', done));
    console.log(JSON.stringify({ event: 'unclaimed-upgrade', errorListeners: socket.listenerCount('error') }));
    resolve({ closed });
  }));
  client = createConnection({ host: '127.0.0.1', port }); client.on('error', () => {});
  await new Promise(resolve => client.once('connect', resolve));
  client.write(`GET /owned-unclaimed-upgrade HTTP/1.1\r\nHost: 127.0.0.1:${port}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n`);
  // Force RST only after Node has handed the raw connection to upgrade hooks.
  const { closed } = await upgraded;
  client.resetAndDestroy();
  await closed;
  const response = await fetch(`http://127.0.0.1:${port}/`);
  if (response.status !== 200 || !(await response.text()).includes('owned vite reset fixture')) throw new Error('HTTP stopped responding after reset');
  const hmr = createConnection({ host: '127.0.0.1', port }); hmr.on('error', () => {});
  const accepted = new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('Authenticated HMR handshake timed out')), 2_000);
    let headers = '';
    hmr.on('data', bytes => {
      headers += bytes.toString();
      if (!headers.includes('\r\n\r\n')) return;
      clearTimeout(timer);
      if (!headers.startsWith('HTTP/1.1 101 ') || !headers.includes('Sec-WebSocket-Protocol: vite-hmr')) reject(new Error('HMR handshake rejected'));
      else resolve();
    });
  });
  await new Promise(resolve => hmr.once('connect', resolve));
  hmr.write(`GET /?token=${server.config.webSocketToken} HTTP/1.1\r\nHost: 127.0.0.1:${port}\r\nOrigin: http://127.0.0.1:${port}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Protocol: vite-hmr\r\n\r\n`);
  await accepted; hmr.destroy();
  console.log(JSON.stringify({ event: 'survived', httpStatus: response.status, authenticatedHmrAccepted: true,
    projectConfig, projectGuardInstalled: server.config.plugins.some(plugin => plugin.name === 'nomifun-dev-upgrade-sockets'),
    globalExceptionListeners: process.listenerCount('uncaughtException'), logs }));
} finally { client?.destroy(); await server.close(); }
