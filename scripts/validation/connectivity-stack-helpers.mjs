import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { createHash, randomBytes } from 'node:crypto';
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import http from 'node:http';
import https from 'node:https';
import net from 'node:net';
import dgram from 'node:dgram';
import { join } from 'node:path';

export const delay = ms => new Promise(resolve => setTimeout(resolve, ms));

export async function until(label, probe, { timeoutMs = 30_000, intervalMs = 150 } = {}) {
  const deadline = Date.now() + timeoutMs;
  let last;
  while (Date.now() < deadline) {
    try { const value = await probe(); if (value) return value; } catch (error) { last = error; }
    await delay(intervalMs);
  }
  throw new Error(`${label} timed out${last ? ` (${last.code ?? last.name})` : ''}`);
}

export async function freePort(protocol = 'tcp', requested = 0) {
  const socket = protocol === 'udp' ? dgram.createSocket('udp4') : net.createServer();
  return new Promise((resolve, reject) => {
    socket.once('error', reject);
    const ready = () => { const port = socket.address().port; socket.close(() => resolve(port)); };
    protocol === 'udp' ? socket.bind(requested, '127.0.0.1', ready) : socket.listen(requested, '127.0.0.1', ready);
  });
}

// No retry, redirect, inherited proxy, cookie jar, or relaxed TLS validation.
// The caller decides whether a failed mutation may be inspected or resubmitted.
export function request(url, { method = 'GET', headers = {}, body, ca, signal, timeoutMs = 10_000 } = {}) {
  const target = new URL(url);
  assert(['http:', 'https:'].includes(target.protocol), 'HTTP(S) URL required');
  const payload = body === undefined ? undefined : (typeof body === 'string' ? body : JSON.stringify(body));
  return new Promise((resolve, reject) => {
    const transport = target.protocol === 'https:' ? https : http;
    const outgoing = transport.request(target, {
      method, ca, rejectUnauthorized: true, signal,
      headers: { ...(payload === undefined ? {} : { 'content-type': 'application/json', 'content-length': Buffer.byteLength(payload) }), ...headers },
    }, incoming => {
      const chunks = [];
      let length = 0;
      incoming.on('data', chunk => { length += chunk.length; if (length > 8 * 1024 * 1024) outgoing.destroy(new Error('Response bound exceeded')); else chunks.push(chunk); });
      incoming.once('error', reject);
      incoming.once('end', () => {
        const text = Buffer.concat(chunks).toString('utf8');
        let json; try { json = text ? JSON.parse(text) : null; } catch { json = null; }
        resolve({ status: incoming.statusCode, headers: incoming.headers, text, json });
      });
    });
    outgoing.setTimeout(timeoutMs, () => outgoing.destroy(new Error('Request timeout')));
    outgoing.once('error', reject);
    outgoing.end(payload);
  });
}

export function fetchAdapter({ ca, observe } = {}) {
  return async (url, init = {}) => {
    observe?.({ url: String(url), method: init.method ?? 'GET' });
    const result = await request(url, { ...init, ca });
    return new Response(result.text, { status: result.status, headers: result.headers });
  };
}

export function certificate(directory, opensslPath) {
  const executable = opensslPath ?? (process.platform === 'win32' && existsSync('C:/Program Files/Git/usr/bin/openssl.exe') ? 'C:/Program Files/Git/usr/bin/openssl.exe' : 'openssl');
  const certPath = join(directory, 'local-ca.pem');
  const keyPath = join(directory, 'local-key.pem');
  const result = spawnSync(executable, ['req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-keyout', keyPath, '-out', certPath,
    '-days', '2', '-subj', '/CN=localhost', '-addext', 'subjectAltName=DNS:localhost,IP:127.0.0.1',
    '-addext', 'basicConstraints=critical,CA:TRUE', '-addext', 'keyUsage=critical,digitalSignature,keyEncipherment,keyCertSign'],
  { stdio: 'pipe', windowsHide: true });
  assert.equal(result.status, 0, 'OpenSSL certificate generation failed (use --openssl-bin)');
  return { cert: readFileSync(certPath), key: readFileSync(keyPath) };
}

export async function startProxy({ target, port = 0, cert, key, upstreamCa, dropResponse, holdResponse }) {
  const sockets = new Set();
  const counters = new Map();
  const upstream = new URL(target);
  const transport = upstream.protocol === 'https:' ? https : http;
  const handler = (incoming, outgoing) => {
    const countKey = `${incoming.method} ${incoming.url}`;
    counters.set(countKey, (counters.get(countKey) ?? 0) + 1);
    const headers = { ...incoming.headers, host: upstream.host };
    delete headers.connection;
    const forward = transport.request(new URL(incoming.url, upstream), { method: incoming.method, headers, ca: upstreamCa, rejectUnauthorized: true }, response => {
      if (dropResponse?.(incoming, response)) { response.resume(); response.once('end', () => outgoing.destroy()); return; }
      const deliver = () => { outgoing.writeHead(response.statusCode, response.headers); response.pipe(outgoing); };
      const held = holdResponse?.(incoming, response);
      held ? Promise.resolve(held).then(deliver, () => outgoing.destroy()) : deliver();
    });
    forward.once('error', () => { if (!outgoing.headersSent) outgoing.writeHead(502); outgoing.end('upstream unavailable'); });
    incoming.pipe(forward);
  };
  const server = cert ? https.createServer({ cert, key }, handler) : http.createServer(handler);
  server.on('connection', socket => { sockets.add(socket); socket.once('close', () => sockets.delete(socket)); });
  server.on('upgrade', (incoming, downstream, head) => {
    const headers = { ...incoming.headers, host: upstream.host };
    const forward = transport.request(new URL(incoming.url, upstream), { headers, ca: upstreamCa, rejectUnauthorized: true });
    forward.once('upgrade', (response, remote, remoteHead) => {
      sockets.add(remote); remote.once('close', () => sockets.delete(remote));
      downstream.write(`HTTP/1.1 101 Switching Protocols\r\n${Object.entries(response.headers).map(([name, value]) => `${name}: ${value}\r\n`).join('')}\r\n`);
      if (head.length) remote.write(head);
      if (remoteHead.length) downstream.write(remoteHead);
      downstream.pipe(remote).pipe(downstream);
      downstream.once('close', () => remote.destroy()); remote.once('close', () => downstream.destroy());
      downstream.on('error', () => remote.destroy()); remote.on('error', () => downstream.destroy());
    });
    forward.once('response', response => { downstream.end(`HTTP/1.1 ${response.statusCode} Rejected\r\nConnection: close\r\n\r\n`); response.resume(); });
    forward.once('error', () => downstream.destroy());
    forward.end();
  });
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(port, '127.0.0.1', resolve); });
  return {
    url: `${cert ? 'https' : 'http'}://127.0.0.1:${server.address().port}`, counters,
    close: async () => { for (const socket of sockets) socket.destroy(); await new Promise(resolve => server.close(resolve)); },
  };
}

function clientFrame(opcode, payload) {
  const data = Buffer.isBuffer(payload) ? payload : Buffer.from(payload);
  assert(data.length < 65_536, 'Client frame bound exceeded');
  const mask = randomBytes(4);
  const header = data.length < 126 ? Buffer.from([0x80 | opcode, 0x80 | data.length]) : Buffer.from([0x80 | opcode, 0xfe, data.length >> 8, data.length & 255]);
  const masked = Buffer.from(data);
  for (let i = 0; i < masked.length; i++) masked[i] ^= mask[i % 4];
  return Buffer.concat([header, mask, masked]);
}

export async function openSocket(baseUrl, token, { ca, timeoutMs = 10_000, headers = {}, cookie, origin } = {}) {
  const target = new URL('/ws', baseUrl);
  const key = randomBytes(16).toString('base64');
  const expected = createHash('sha1').update(`${key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11`).digest('base64');
  const transport = target.protocol === 'https:' ? https : http;
  return new Promise((resolve, reject) => {
    const outgoing = transport.request(target, { ca, rejectUnauthorized: true, headers: {
      Connection: 'Upgrade', Upgrade: 'websocket', 'Sec-WebSocket-Version': '13', 'Sec-WebSocket-Key': key,
      ...(token ? { 'Sec-WebSocket-Protocol': token } : {}), ...(cookie ? { Cookie: cookie } : {}), ...(origin ? { Origin: origin } : {}), ...headers,
    } });
    const timer = setTimeout(() => outgoing.destroy(new Error('WS upgrade timeout')), timeoutMs);
    outgoing.once('error', error => { clearTimeout(timer); reject(error); });
    outgoing.once('response', incoming => { clearTimeout(timer); incoming.resume(); reject(Object.assign(new Error('WS upgrade rejected'), { status: incoming.statusCode })); });
    outgoing.once('upgrade', (response, socket, head) => {
      clearTimeout(timer);
      if (response.headers['sec-websocket-accept'] !== expected) { socket.destroy(); reject(new Error('Invalid WS accept')); return; }
      let buffer = Buffer.alloc(0), fragment = [], fragmentOpcode;
      const result = { messages: [], closed: false, closeCode: null, error: null, close: () => socket.destroy(), send: value => socket.write(clientFrame(1, JSON.stringify(value))) };
      const receive = chunk => {
        buffer = Buffer.concat([buffer, chunk]);
        while (buffer.length >= 2) {
          const fin = Boolean(buffer[0] & 128), opcode = buffer[0] & 15;
          let length = buffer[1] & 127, offset = 2;
          if (length === 126) { if (buffer.length < 4) return; length = buffer.readUInt16BE(2); offset = 4; }
          if (length === 127) { if (buffer.length < 10) return; const large = buffer.readBigUInt64BE(2); if (large > 8n * 1024n * 1024n) { socket.destroy(); return; } length = Number(large); offset = 10; }
          if (buffer[1] & 128) { socket.destroy(); return; } // Server frames must be unmasked.
          if (buffer.length < offset + length) return;
          const data = buffer.subarray(offset, offset + length); buffer = buffer.subarray(offset + length);
          if (opcode === 8) { result.closeCode = data.length >= 2 ? data.readUInt16BE(0) : null; socket.end(clientFrame(8, data)); continue; }
          if (opcode === 9) { socket.write(clientFrame(10, data)); continue; }
          if (opcode === 10) continue;
          if (opcode !== 0) { fragmentOpcode = opcode; fragment = []; }
          fragment.push(data);
          if (!fin) continue;
          if (fragmentOpcode === 1) {
            const text = Buffer.concat(fragment).toString('utf8');
            try {
              const message = JSON.parse(text); result.messages.push(message);
              if (message.name === 'ping') result.send({ name: 'pong', data: { timestamp: message.data?.timestamp ?? Date.now() } });
            } catch { result.error = 'invalid_json'; }
          }
          fragment = [];
        }
      };
      socket.on('data', receive); socket.on('error', error => { result.error = error.code ?? error.name; }); socket.once('close', () => { result.closed = true; });
      if (head.length) receive(head);
      resolve(result);
    });
    outgoing.end();
  });
}

// The production realtime handler deliberately upgrades before it validates
// credentials: authentication failure is auth-expired followed by close 1008.
// A 101 alone neither grants access nor proves that invalid credentials worked.
export async function assertSocketAuthenticationRejected(baseUrl, token, options = {}) {
  let socket;
  try {
    try { socket = await openSocket(baseUrl, token, options); }
    catch (error) { assert([401, 403].includes(error.status), 'Expected an authentication rejection, not a network failure'); return { upgrade_status: error.status }; }
    await until('unauthorized socket closes without business publication', () => socket.closed, { timeoutMs: 5_000 });
    assert.equal(socket.closeCode, 1008, 'Unauthorized socket must close for authentication policy');
    assert(socket.messages.length > 0 && socket.messages.every(message => message.name === 'auth-expired'), 'Unauthorized socket published a business or success frame');
    return { upgrade_status: 101, auth_expired_received: true, close_code: socket.closeCode, business_frames: 0 };
  } finally { socket?.close(); }
}

export class Processes {
  children = new Set();
  secrets = new Set();
  constructor(directory) { this.directory = directory; }
  redact(value) { let safe = value; for (const secret of this.secrets) if (secret) safe = safe.replaceAll(secret, '[redacted]'); return safe.replace(/eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+/g, '[jwt]'); }
  start(label, executable, args, env = {}, options = {}) {
    const child = spawn(executable, args, { cwd: this.directory, env: { ...process.env, ...env }, stdio: [options.rpc ? 'pipe' : 'ignore', 'pipe', 'pipe'], windowsHide: true });
    let output = '';
    const capture = data => { output += data.toString(); if (output.length > 128 * 1024) output = output.slice(-128 * 1024); };
    if (!options.rpc) child.stdout.on('data', capture); child.stderr.on('data', capture);
    child.on('error', error => { output += `${error.code ?? error.name}\n`; });
    const owned = { child, label, output: () => this.redact(output) };
    this.children.add(owned);
    child.once('exit', () => this.children.delete(owned));
    return owned;
  }
  async stop(owned) {
    if (owned.child.exitCode !== null || owned.child.signalCode !== null || !this.children.has(owned)) return;
    // All executables here are launched directly: never kill a name, port, or unknown PID.
    owned.child.kill('SIGTERM');
    const exited = () => owned.child.exitCode !== null || owned.child.signalCode !== null;
    try { await until('owned process exit', exited, { timeoutMs: 5_000 }); } catch { owned.child.kill('SIGKILL'); await until('owned process termination', exited, { timeoutMs: 5_000 }); }
  }
  async close() {
    const errors = [];
    for (const owned of [...this.children].reverse()) try { await this.stop(owned); } catch (error) { errors.push(error); }
    if (errors.length) throw new AggregateError(errors, 'Owned process cleanup failed');
  }
  log(owned) { writeFileSync(join(this.directory, `${owned.label}.log`), owned.output()); }
}

export function stdioRpc(owned, { timeoutMs = 60_000, secrets } = {}) {
  assert(owned.child.stdin, 'RPC child must have owned stdin pipe');
  let sequence = 0, buffer = '';
  const metrics = { stdout_bytes: 0, json_lines: 0, non_json_lines: 0, matched_responses: 0, unmatched_responses: 0, last_response_id: null };
  const pending = new Map();
  const rejectAll = () => { for (const entry of pending.values()) { clearTimeout(entry.timer); entry.reject(new Error('Owned RPC process exited')); } pending.clear(); };
  owned.child.once('exit', rejectAll); owned.child.once('error', rejectAll);
  owned.child.stdout.on('data', chunk => {
    metrics.stdout_bytes += chunk.length;
    buffer += chunk.toString();
    if (buffer.length > 128 * 1024) { rejectAll(); owned.child.kill(); return; }
    while (buffer.includes('\n')) {
      const index = buffer.indexOf('\n'), line = buffer.slice(0, index); buffer = buffer.slice(index + 1);
      let response; try { response = JSON.parse(line); } catch { metrics.non_json_lines++; continue; }
      metrics.json_lines++; metrics.last_response_id = typeof response.id === 'number' ? response.id : null;
      const entry = pending.get(response.id); if (!entry) { metrics.unmatched_responses++; continue; }
      metrics.matched_responses++;
      pending.delete(response.id); clearTimeout(entry.timer);
      if (response.result?.nativeTrust) secrets?.add(response.result.nativeTrust);
      if (response.result?.pairUrl) secrets?.add(response.result.pairUrl);
      response.ok ? entry.resolve(response.result) : entry.reject(new Error(`Native RPC ${entry.method}: ${response.error ?? 'unknown error'}`));
    }
  });
  return { diagnostics: () => ({ ...metrics }), call(method, params = {}, waitMs = timeoutMs) {
    return new Promise((resolve, reject) => {
      const id = ++sequence;
      const timer = setTimeout(() => { pending.delete(id); reject(new Error(`Native RPC ${method} timed out; request_id=${id}; metadata=${JSON.stringify(metrics)}`)); }, waitMs);
      pending.set(id, { resolve, reject, timer, method });
      owned.child.stdin.write(`${JSON.stringify({ id, method, ...params })}\n`, error => {
        if (error && pending.has(id)) { clearTimeout(timer); pending.delete(id); reject(error); }
      });
    });
  } };
}
