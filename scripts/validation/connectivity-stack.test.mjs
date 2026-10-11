import assert from 'node:assert/strict';
import { createHash, randomBytes } from 'node:crypto';
import { mkdtempSync, rmSync } from 'node:fs';
import http from 'node:http';
import https from 'node:https';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { after, test } from 'node:test';
import { assertSocketAuthenticationRejected, certificate, freePort, openSocket, Processes, request, startProxy, stdioRpc, until } from './connectivity-stack-helpers.mjs';

const fixture = mkdtempSync(join(tmpdir(), 'nomifun-connectivity-helper-'));
after(() => rmSync(fixture, { recursive: true, force: true }));

async function listen(server) { await new Promise(resolve => server.listen(0, '127.0.0.1', resolve)); return `http://127.0.0.1:${server.address().port}`; }
async function close(server) { server.closeAllConnections?.(); await new Promise(resolve => server.close(resolve)); }

test('HTTPS requests verify explicit CA and reject untrusted certificate chains', async () => {
  const tls = certificate(fixture);
  const server = https.createServer(tls, (_, outgoing) => outgoing.end('{"real_tls":true}'));
  const address = (await listen(server)).replace('http:', 'https:');
  try {
    await assert.rejects(request(address));
    assert.equal((await request(address, { ca: tls.cert })).json.real_tls, true);
  } finally { await close(server); }
});

test('lost POST acknowledgement never causes the runner to resend a real upstream mutation', async () => {
  let mutations = 0;
  const server = http.createServer(async (incoming, outgoing) => {
    for await (const chunk of incoming) { void chunk; }
    mutations++;
    outgoing.end('{"accepted":true}');
  });
  const upstream = await listen(server);
  const proxy = await startProxy({ target: upstream, dropResponse: incoming => incoming.method === 'POST' });
  try {
    await assert.rejects(request(`${proxy.url}/mutation`, { method: 'POST', body: { id: 'one-operation' } }));
    assert.equal(mutations, 1);
    assert.equal(proxy.counters.get('POST /mutation'), 1);
  } finally { await proxy.close(); await close(server); }
});

test('HTTPS proxy carries real authenticated WebSocket upgrades and close frames', async () => {
  const sockets = new Set();
  const server = http.createServer();
  server.on('connection', socket => { sockets.add(socket); socket.once('close', () => sockets.delete(socket)); });
  server.on('upgrade', (incoming, socket) => {
    assert.equal(incoming.headers['sec-websocket-protocol'], 'synthetic-device-credential');
    assert.equal(incoming.headers.cookie, undefined);
    const accept = createHash('sha1').update(`${incoming.headers['sec-websocket-key']}258EAFA5-E914-47DA-95CA-C5AB0DC85B11`).digest('base64');
    socket.write(`HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Accept: ${accept}\r\n\r\n`);
    const message = Buffer.from('{"name":"wire.accepted","data":{"ok":true}}');
    socket.write(Buffer.concat([Buffer.from([0x81, message.length]), message]));
    setTimeout(() => socket.end(Buffer.from([0x88, 2, 0x03, 0xf0])), 60); // 1008 policy rejection.
  });
  const upstream = await listen(server), tls = certificate(fixture);
  const proxy = await startProxy({ target: upstream, ...tls });
  try {
    await assert.rejects(openSocket(proxy.url, 'synthetic-device-credential'));
    const client = await openSocket(proxy.url, 'synthetic-device-credential', { ca: tls.cert });
    await until('authenticated frame and close', () => client.closed && client.messages.length > 0);
    assert.equal(client.messages[0].name, 'wire.accepted');
    assert.equal(client.closeCode, 1008);
  } finally { await proxy.close(); for (const socket of sockets) socket.destroy(); await close(server); }
});

test('a 101 upgrade followed by auth-expired/1008 rejects access, but any business publication fails the assertion', async () => {
  const sockets = new Set();
  const server = http.createServer();
  server.on('connection', socket => { sockets.add(socket); socket.once('close', () => sockets.delete(socket)); });
  server.on('upgrade', (incoming, socket) => {
    const accept = createHash('sha1').update(`${incoming.headers['sec-websocket-key']}258EAFA5-E914-47DA-95CA-C5AB0DC85B11`).digest('base64');
    socket.write(`HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Accept: ${accept}\r\n\r\n`);
    const publish = value => { const text = Buffer.from(JSON.stringify(value)); socket.write(Buffer.concat([Buffer.from([0x81, text.length]), text])); };
    if (incoming.headers['sec-websocket-protocol'] === 'leaking.payload.signature') publish({ name: 'private', data: 'must not pass' });
    publish({ name: 'auth-expired', data: { message: 'invalid token' } });
    socket.end(Buffer.from([0x88, 2, 0x03, 0xf0]));
  });
  const address = await listen(server);
  try {
    const evidence = await assertSocketAuthenticationRejected(address, 'invalid.payload.signature');
    assert.equal(evidence.upgrade_status, 101); assert.equal(evidence.close_code, 1008); assert.equal(evidence.business_frames, 0);
    await assert.rejects(assertSocketAuthenticationRejected(address, 'leaking.payload.signature'), /published a business/);
  } finally { for (const socket of sockets) socket.destroy(); await close(server); }
});

test('an occupied reserved port is detected without terminating its owner', async () => {
  const server = http.createServer((_, response) => response.end('{"alive":true}'));
  const address = await listen(server);
  try {
    await assert.rejects(freePort('tcp', Number(new URL(address).port)), error => error.code === 'EADDRINUSE');
    assert.equal((await request(address)).json.alive, true);
  } finally { await close(server); }
});

test('cleanup stops only directly spawned child handles and redacts their credentials', async () => {
  const owner = new Processes(fixture), secret = randomBytes(20).toString('hex'); owner.secrets.add(secret);
  const child = owner.start('owned-helper', process.execPath, ['-e', `process.stdout.write(process.env.CONNECTIVITY_TEST_VALUE);setInterval(()=>{},1000)`], { CONNECTIVITY_TEST_VALUE: secret });
  await until('owned child stdout', () => child.output().includes('[redacted]'));
  assert(!child.output().includes(secret));
  await owner.close();
  assert.equal(owner.children.size, 0);
  assert(child.child.exitCode !== null || child.child.signalCode !== null);
});

test('private RPC ignores console lines, matches numeric ids and reports metadata without saving trust payloads', async () => {
  const owner = new Processes(fixture), secret = randomBytes(20).toString('hex');
  const source = 'const readline=require("node:readline");process.stdout.write("console startup\\n");readline.createInterface({input:process.stdin}).on("line",line=>{const req=JSON.parse(line);process.stdout.write(JSON.stringify({id:req.id,ok:true,result:{nativeTrust:process.env.CONNECTIVITY_TEST_TRUST}})+"\\n")});';
  const child = owner.start('private-rpc-helper', process.execPath, ['-e', source], { CONNECTIVITY_TEST_TRUST: secret }, { rpc: true });
  const rpc = stdioRpc(child, { secrets: owner.secrets });
  try {
    const result = await rpc.call('boot'); assert.equal(result.nativeTrust, secret);
    const diagnostics = rpc.diagnostics();
    assert.equal(diagnostics.non_json_lines, 1); assert.equal(diagnostics.matched_responses, 1); assert.equal(diagnostics.last_response_id, 1);
    assert(!JSON.stringify(diagnostics).includes(secret)); assert.equal(child.output(), ''); assert(owner.secrets.has(secret));
  } finally { await owner.close(); }
});
