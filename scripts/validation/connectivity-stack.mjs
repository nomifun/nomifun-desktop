#!/usr/bin/env node
/** Real, disposable Desktop + NomiRelay + nfagent + Mobile-client acceptance. */
import assert from 'node:assert/strict';
import { createHash, randomBytes } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import http from 'node:http';
import dgram from 'node:dgram';
import { tmpdir } from 'node:os';
import { dirname, isAbsolute, join, relative, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { assertSocketAuthenticationRejected, certificate, delay, fetchAdapter, freePort, openSocket, Processes, request, startProxy, until } from './connectivity-stack-helpers.mjs';
import { nativeAcceptance } from './connectivity-stack-native.mjs';
import { expiredBootstrapAcceptance } from './connectivity-stack-recovery.mjs';
import { runNativeOnly } from './connectivity-stack-native-only.mjs';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const RESERVED_DESKTOP_PORT = 25811;
const REPLY = 'CONNECTIVITY_STACK_CANONICAL_REPLY';
const INPUT = 'connectivity stack isolated text turn';
const randomId = () => randomBytes(32).toString('hex');
const bearer = token => ({ Authorization: `Bearer ${token}` });

function sourceRevision(directory) {
  const git = args => spawnSync('git', ['-C', directory, ...args], { encoding: 'utf8', windowsHide: true });
  const commit = git(['rev-parse', 'HEAD']);
  const status = git(['status', '--porcelain']);
  return {
    directory,
    commit: commit.status === 0 ? commit.stdout.trim() : null,
    worktree_modified: status.status === 0 ? Boolean(status.stdout.trim()) : null,
  };
}

export function parseArguments(args) {
  const values = { mobileRoot: resolve(ROOT, '../nomifun-mobile'), output: resolve(ROOT, '.tmp-connectivity-stack'), keepFixture: false, nativeOnly: false };
  const options = new Map([['--desktop-bin', 'desktopBin'], ['--relay-bin', 'relayBin'], ['--agent-bin', 'agentBin'], ['--mobile-root', 'mobileRoot'], ['--output', 'output'], ['--openssl-bin', 'opensslBin'], ['--native-harness-bin', 'nativeHarnessBin'], ['--native-spa-dir', 'nativeSpaDir'], ['--managed-agent-bin', 'managedAgentBin']]);
  for (let i = 0; i < args.length; i++) {
    if (args[i] === '--help') return { help: true };
    if (args[i] === '--native-only') { values.nativeOnly = true; continue; }
    if (args[i] === '--keep-fixture') { values.keepFixture = true; continue; }
    const name = options.get(args[i]);
    assert(name && args[i + 1] && !args[i + 1].startsWith('--'), `Unknown or missing option: ${args[i]}`);
    values[name] = resolve(args[++i]);
  }
  for (const name of values.nativeOnly ? ['relayBin', 'nativeHarnessBin'] : ['desktopBin', 'relayBin', 'agentBin']) assert(values[name] && existsSync(values[name]), `Provide existing --${name.replace('Bin', '-bin')}`);
  if (values.nativeOnly && !args.includes('--output')) values.output = resolve(ROOT, '.tmp-connectivity-stack-native-only');
  if (!values.nativeOnly) assert(existsSync(join(values.mobileRoot, 'src/api')), '--mobile-root must be the real Mobile checkout');
  if (values.nativeHarnessBin) assert(existsSync(values.nativeHarnessBin), '--native-harness-bin must exist');
  if (values.managedAgentBin) assert(values.nativeHarnessBin && existsSync(values.managedAgentBin), '--managed-agent-bin requires an existing native harness and agent binary');
  return values;
}

const HELP = `Real local connectivity acceptance (Node 24+ required).

node scripts/validation/connectivity-stack.mjs --desktop-bin <nomifun-web.exe> --relay-bin <nfrelay.exe> --agent-bin <nfagent.exe> [--mobile-root ../nomifun-mobile] [--output .tmp-connectivity-stack] [--keep-fixture] [--openssl-bin <openssl.exe>] [--native-harness-bin <connection_lifecycle_harness.exe> --native-spa-dir ui/dist --managed-agent-bin <manifest-pinned-nfagent.exe>]

Native diagnosis without repeating core: --native-only --relay-bin <nfrelay.exe> --native-harness-bin <connection_lifecycle_harness.exe> [--native-spa-dir ui/dist --managed-agent-bin <manifest-pinned-nfagent.exe>]. This diagnostic subset does not replace a full green delivery run.

Build current sources before running: cargo build -p nomifun-web; go build ./cmd/nfrelay; go build ./cmd/nfagent in the Relay checkout. Optional native harness: cargo build -p nomifun-desktop --example connection_lifecycle_harness --no-default-features. This runner never builds, kills a port owner, or uses production data. Desktop port 25811 must be free; other listener ports are allocated on loopback. Every launched process receives a fresh data directory. The real Relay HTTPS/WSS gateway uses an explicitly trusted test CA; the real agent also validates its SPKI pin. Only the external model response is synthetic. Native acceptance uses the production connection manager and the shipped SHA-pinned runtime (provided matching binary or official download). No public routing, phone hardware, Tauri window UI, ACME, throughput or HA claim is made.
`;

function unwrap(result, label = 'API request', expected = 200) {
  assert.equal(result.status, expected, `${label}: HTTP ${result.status}`);
  assert(result.json !== null, `${label}: JSON expected`);
  assert(result.json.success !== false, `${label}: product error ${result.json.error?.code ?? result.json.code ?? 'unknown'}`);
  return result.json.data ?? result.json;
}

async function modelFixture() {
  const calls = [];
  const sockets = new Set();
  const server = http.createServer(async (incoming, outgoing) => {
    if (incoming.method !== 'POST' || incoming.url !== '/v1/chat/completions') { outgoing.writeHead(404).end(); return; }
    let body = ''; for await (const chunk of incoming) { body += chunk; if (body.length > 2 * 1024 * 1024) { outgoing.writeHead(413).end(); return; } }
    const parsed = JSON.parse(body);
    calls.push({ model: parsed.model, messageCount: parsed.messages?.length ?? 0, stream: parsed.stream === true });
    assert.equal(parsed.model, 'connectivity-fixture');
    outgoing.writeHead(200, { 'content-type': 'text/event-stream', 'cache-control': 'no-cache' });
    const frame = (delta, finish_reason = null) => `data: ${JSON.stringify({ id: 'connectivity-fixture', object: 'chat.completion.chunk', created: 1, model: 'connectivity-fixture', choices: [{ index: 0, delta, finish_reason }] })}\n\n`;
    outgoing.write(frame({ role: 'assistant', content: REPLY.slice(0, 12) }));
    await delay(80);
    outgoing.write(frame({ content: REPLY.slice(12) }));
    await delay(80);
    outgoing.end(`${frame({}, 'stop')}data: [DONE]\n\n`);
  });
  server.on('connection', socket => { sockets.add(socket); socket.once('close', () => sockets.delete(socket)); });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  return { url: `http://127.0.0.1:${server.address().port}`, calls, close: async () => { for (const socket of sockets) socket.destroy(); await new Promise(resolve => server.close(resolve)); } };
}

function childEnvironment(fixture, password) {
  return {
    NOMIFUN_WEB_INSECURE_NO_AUTH: 'false', NOMIFUN_DATA_DIR: fixture,
    NOMIFUN_WORK_DIR: join(fixture, 'work'), NOMIFUN_CACHE_DIR: join(fixture, 'cache'), NOMIFUN_LOG_DIR: join(fixture, 'logs'),
    NOMIFUN_ADMIN_USERNAME: 'connectivity-owner', NOMIFUN_ADMIN_PASSWORD: password,
    NO_PROXY: '127.0.0.1,localhost', no_proxy: '127.0.0.1,localhost',
  };
}

export async function run(config) {
  assert(!process.versions.bun && Number(process.versions.node.split('.')[0]) >= 24, 'Use Node 24+ for strict Node HTTP/WS behavior and native TypeScript stripping');
  if (config.nativeOnly) return runNativeOnly(config, ROOT);
  // Probe only. A port conflict fails before spawning anything and never kills the owner.
  await freePort('tcp', RESERVED_DESKTOP_PORT);
  const temporaryParent = realpathSync(tmpdir());
  const fixture = mkdtempSync(join(temporaryParent, 'nomifun-connectivity-'));
  const marker = randomId();
  writeFileSync(join(fixture, '.runner-owned'), marker);
  mkdirSync(config.output, { recursive: true });
  const processes = new Processes(fixture);
  const allChildren = [], resources = [], sockets = [];
  const password = `Local-${randomId()}!`;
  processes.secrets.add(password);
  const report = {
    schema_version: 1, started_at: new Date().toISOString(), result: 'running',
    sources: {
      desktop: sourceRevision(ROOT),
      relay: sourceRevision(resolve(ROOT, '../nomifun-net-infra')),
      mobile: sourceRevision(config.mobileRoot),
    },
    environment: 'same-computer-loopback', evidence_source: 'real-product-binaries-and-mobile-client',
    synthetic: ['controlled OpenAI-compatible model response', 'test CA loaded by real Relay TLS gateway'],
    boundaries: ['public routing/NAT/firewall not measured', 'Tauri shell/native lifecycle not measured', 'Android/iOS device camera and deep links not measured', 'ACME/production certificate lifecycle not measured', 'capacity/HA not measured'],
    binaries: Object.fromEntries(['desktopBin', 'relayBin', 'agentBin'].map(name => [name, { path: config[name], sha256: createHash('sha256').update(readFileSync(config[name])).digest('hex') }])),
    checks: [], processes_cleaned: false, fixture_removed: false,
  };
  const check = async (name, action) => {
    process.stdout.write(`CHECK ${name}\n`);
    const started = Date.now();
    try { const evidence = await action(); report.checks.push({ name, result: 'pass', duration_ms: Date.now() - started, evidence }); return evidence; }
    catch (error) { report.checks.push({ name, result: 'fail', duration_ms: Date.now() - started, error: processes.redact(error.message) }); throw error; }
  };
  const launch = (label, binary, args, env = {}) => { const owned = processes.start(label, binary, args, env); allChildren.push(owned); return owned; };
  const cleanup = async () => {
    const failures = [];
    for (const socket of sockets) socket.close();
    for (const resource of [...resources].reverse()) try { await resource.close(); } catch (error) { failures.push(error); }
    try { await processes.close(); } catch (error) { failures.push(error); }
    report.processes_cleaned = processes.children.size === 0;
    for (const child of allChildren) writeFileSync(join(config.output, `${child.label}.log`), child.output());
    if (!config.keepFixture && failures.length === 0) {
      const actual = realpathSync(fixture), within = relative(temporaryParent, actual);
      assert(within && !within.startsWith('..') && !isAbsolute(within) && readFileSync(join(actual, '.runner-owned'), 'utf8') === marker, 'Fixture ownership check failed');
      rmSync(actual, { recursive: true, force: true });
      report.fixture_removed = true;
    } else report.fixture = fixture;
    if (failures.length) throw new AggregateError(failures, 'Cleanup failed; fixture retained for diagnosis');
  };
  try {
    const desktopDirectory = join(fixture, 'desktop'); mkdirSync(desktopDirectory);
    const desktopProcess = launch('desktop', config.desktopBin, ['--host', '127.0.0.1', '--port', String(RESERVED_DESKTOP_PORT), '--api-only', '--data-dir', desktopDirectory], childEnvironment(desktopDirectory, password));
    const desktopUrl = `http://127.0.0.1:${RESERVED_DESKTOP_PORT}`;
    await until('authenticated Desktop startup', async () => (await request(`${desktopUrl}/api/auth/status`)).status === 200, { timeoutMs: 90_000 });
    const ownerLogin = await request(`${desktopUrl}/login`, { method: 'POST', body: { username: 'connectivity-owner', password } });
    const login = unwrap(ownerLogin, 'owner login');
    const ownerToken = login.token; assert(ownerToken, 'Owner login JWT missing'); processes.secrets.add(ownerToken);
    const ownerCookie = (ownerLogin.headers['set-cookie'] ?? []).map(cookie => cookie.split(';')[0]).filter(cookie => cookie.startsWith('nomifun-session=')).join('; ');
    assert(ownerCookie, 'Real owner browser session cookie missing'); processes.secrets.add(ownerCookie);
    const ownerRequest = (path, options = {}) => request(`${desktopUrl}${path}`, { ...options, headers: { ...bearer(ownerToken), ...options.headers } });
    await check('desktop_requires_authentication', async () => {
      const denied = await request(`${desktopUrl}/api/agent-sessions`);
      assert([401, 403].includes(denied.status));
      return { status: denied.status, port: RESERVED_DESKTOP_PORT };
    });

    const udpPort = await freePort('udp'), consolePort = await freePort(), businessPort = await freePort();
    const consoleUrl = `http://127.0.0.1:${consolePort}`;
    const tls = certificate(fixture, config.opensslBin);
    const tlsEntry = { url: `https://localhost:${businessPort}` };
    const relayDirectory = join(fixture, 'relay'); mkdirSync(relayDirectory);
    writeFileSync(join(relayDirectory, 'relay-cert.pem'), tls.cert);
    writeFileSync(join(relayDirectory, 'relay-key.pem'), tls.key, { mode: 0o600 });
    launch('relay', config.relayBin, ['-data-dir', relayDirectory, '-bind', `127.0.0.1:${udpPort}`, '-advertise', `127.0.0.1:${udpPort}`, '-console', `127.0.0.1:${consolePort}`, '-shared', `127.0.0.1:${businessPort}`, '-desktop-bootstrap-url', `${consoleUrl}/api/bootstrap/desktop/v2`, '-desktop-dev-insecure', '-drain', '1s']);
    await until('Relay startup', async () => (await request(`${consoleUrl}/healthz`)).status === 200);
    const setup = await request(`${consoleUrl}/api/setup`, { method: 'POST', headers: { Origin: consoleUrl }, body: { username: 'connectivity-owner', password } });
    const relayLogin = unwrap(setup, 'Relay setup');
    const cookies = (setup.headers['set-cookie'] ?? []).map(cookie => cookie.split(';')[0]).join('; ');
    assert(cookies && relayLogin.csrf_token, 'Relay admin cookie/CSRF missing');
    processes.secrets.add(relayLogin.csrf_token); processes.secrets.add(cookies);
    const relayAdmin = (path, options = {}) => request(`${consoleUrl}${path}`, { ...options, headers: { Cookie: cookies, Origin: consoleUrl, 'X-NFR-CSRF': relayLogin.csrf_token, ...options.headers } });
    const invite = unwrap(await relayAdmin('/api/pairing-invites/v2', { method: 'POST', body: { listen_port: businessPort, business_url: tlsEntry.url, note: 'local-stack-desktop' } }), 'Relay invite');
    processes.secrets.add(invite.token);
    const attempt = randomId();
    const bootstrap = unwrap(await request(invite.bootstrap_url, { method: 'POST', headers: bearer(invite.token), body: { attempt_id: attempt } }), 'Relay bootstrap');
    for (const secret of [bootstrap.enrol_token, bootstrap.control_token]) processes.secrets.add(secret);
    assert.equal(bootstrap.tunnel?.backend?.addr, '127.0.0.1:25811', 'Relay must point at dedicated authenticated listener');
    assert.equal(bootstrap.business_url, tlsEntry.url);
    assert.equal(bootstrap.l2_port, businessPort, 'Relay must declare its actual TLS/TCP carrier port');
    await check('bootstrap_same_attempt_recovers_without_duplicate_resources', async () => {
      const recovered = unwrap(await request(invite.bootstrap_url, { method: 'POST', headers: bearer(invite.token), body: { attempt_id: attempt } }));
      assert.equal(recovered.tunnel?.id, bootstrap.tunnel?.id); assert.equal(recovered.control_token, bootstrap.control_token);
      const conflict = await request(invite.bootstrap_url, { method: 'POST', headers: bearer(invite.token), body: { attempt_id: randomId() } });
      assert(conflict.status >= 400, 'Different attempt must not consume/duplicate existing invite');
      return { same_tunnel: true, conflicting_attempt_status: conflict.status };
    });
    const agentDirectory = join(fixture, 'agent'); mkdirSync(agentDirectory);
    const agentArgs = ['-relay', bootstrap.relay, '-pin', bootstrap.pin, '-token', bootstrap.enrol_token, '-state-dir', agentDirectory, '-l2-port', '0'];
    let agent = launch('agent-first', config.agentBin, agentArgs);
    const publicRequest = (path, options = {}) => request(`${tlsEntry.url}${path}`, { ...options, ca: tls.cert });
    await until('real agent HTTP tunnel', async () => (await publicRequest('/api/connections/info')).status === 200, { timeoutMs: 45_000 });
    const info = unwrap(await publicRequest('/api/connections/info'));
    await check('strict_tls_and_real_agent_tunnel', async () => {
      await assert.rejects(request(`${tlsEntry.url}/api/connections/info`), 'Untrusted test CA must fail TLS');
      assert.equal(info.protocol_version, 1); assert(info.desktop_id, 'Handshake Desktop identity missing');
      const agents = unwrap(await relayAdmin('/api/agents'));
      assert(agents.some(agent => agent.online && agent.rung === 'L0/QUIC'), 'Real Relay must confirm QUIC carrier');
      return { tls_chain_verified_with_explicit_ca: true, untrusted_chain_rejected: true, desktop_id: info.desktop_id, protocol_version: info.protocol_version, relay_reported_carrier: 'L0/QUIC' };
    });

    const { transportModule, connectionModule, contractModule, pairingModule } = await importMobileModules(config.mobileRoot);

    const pair = async name => {
      const ticket = unwrap(await ownerRequest('/api/connections/pairing', { method: 'POST', body: {} }), 'device ticket');
      assert(ticket.qr_token && ticket.ticket_id && ticket.expires_at_ms > Date.now());
      processes.secrets.add(ticket.qr_token);
      const incompatible = await publicRequest('/api/auth/device-pair', { method: 'POST', body: { qr_token: ticket.qr_token, device_name: name, protocol_version: 999 } });
      assert.equal(incompatible.status, 426);
      assert.equal(unwrap(await ownerRequest(`/api/connections/pairing/${ticket.ticket_id}`)).state, 'ready', 'Incompatible client must not consume ticket');
      const envelope = `nomifun://pair?v=1&url=${encodeURIComponent(`${tlsEntry.url}/device-pair?token=${ticket.qr_token}`)}`;
      processes.secrets.add(envelope);
      const pairingStore = connectionModule.createConnectionStore({ read: async () => null, write: async () => {} });
      const pairingEpoch = pairingStore.epoch();
      let redeemed;
      const receipt = await pairingModule.redeemConnectionPayload(envelope, pairingEpoch, { redeem: async (parsed, expectedEpoch) => {
        assert.equal(expectedEpoch, pairingEpoch); assert.equal(pairingStore.epoch(), expectedEpoch);
        assert.equal(parsed.credentialKind, 'device-pair'); assert.equal(parsed.baseUrl, tlsEntry.url); assert.equal(parsed.qrToken, ticket.qr_token);
        redeemed = unwrap(await publicRequest('/api/auth/device-pair', { method: 'POST', body: { qr_token: parsed.qrToken, device_name: name, protocol_version: 1 } }), 'device pair');
        assert(await pairingStore.connect({ baseUrl: parsed.baseUrl, token: redeemed.token, user: redeemed.user, device: redeemed.device, desktop: info }, false, expectedEpoch));
        return { user: redeemed.user, epoch: pairingStore.epoch() };
      } });
      assert.equal(receipt.epoch, pairingStore.epoch()); assert.equal(receipt.user.user_id, redeemed.user.user_id);
      assert(redeemed.token && redeemed.device?.device_id); processes.secrets.add(redeemed.token);
      const consumed = await publicRequest('/api/auth/device-pair', { method: 'POST', body: { qr_token: ticket.qr_token, device_name: `${name}-duplicate`, protocol_version: 1 } });
      assert(consumed.status >= 400, 'Pairing ticket must be one-shot');
      const status = unwrap(await ownerRequest(`/api/connections/pairing/${ticket.ticket_id}`));
      assert.equal(status.state, 'used');
      return { token: redeemed.token, device: redeemed.device, user: redeemed.user, ticket_id: ticket.ticket_id, expires_at_ms: ticket.expires_at_ms };
    };
    const deviceA = await pair('local-device-a'), deviceB = await pair('local-device-b');
    await check('two_independent_single_use_device_pairs', async () => {
      assert.notEqual(deviceA.device.device_id, deviceB.device.device_id);
      return { distinct_devices: true, consumed_ticket_state: 'used', incompatible_protocol_preserves_ticket: true, mobile_pairing_envelope_used: true, expires_in_ms: deviceA.expires_at_ms - Date.now() };
    });

    await check('manual_mobile_login_issues_individually_revocable_device', async () => {
      const loggedIn = unwrap(await publicRequest('/api/auth/device-login', { method: 'POST', body: { username: 'connectivity-owner', password, device_name: 'local-password-device-c', protocol_version: 1 } }));
      assert(loggedIn.token && loggedIn.device?.device_id); processes.secrets.add(loggedIn.token);
      const denied = await publicRequest('/api/connections/devices', { headers: bearer(loggedIn.token) }); assert.equal(denied.status, 403);
      const deniedPair = await publicRequest('/api/connections/pairing', { method: 'POST', headers: bearer(loggedIn.token), body: {} }); assert.equal(deniedPair.status, 403);
      unwrap(await publicRequest('/api/system/info', { headers: bearer(loggedIn.token) }));
      unwrap(await publicRequest('/api/connections/device', { method: 'DELETE', headers: bearer(loggedIn.token) }));
      const revoked = await publicRequest('/api/system/info', { headers: bearer(loggedIn.token) }); assert([401, 403].includes(revoked.status));
      const other = await publicRequest('/api/system/info', { headers: bearer(deviceB.token) }); assert.equal(other.status, 200);
      return { password_login_creates_device: true, device_cannot_manage_other_credentials: true, self_revoke_works: true, other_device_alive: true };
    });

    // Load the same platform-neutral transport used by Mobile; it receives real wire I/O.
    const mobile = await createMobileClient({ transportModule, connectionModule, contractModule, baseUrl: tlsEntry.url, device: deviceB, info, ca: tls.cert });
    const socketA = await openSocket(tlsEntry.url, deviceA.token, { ca: tls.cert }); sockets.push(socketA);
    const socketB = await openSocket(tlsEntry.url, deviceB.token, { ca: tls.cert }); sockets.push(socketB);
    const cookieSocketA = await openSocket(tlsEntry.url, deviceA.token, { ca: tls.cert, cookie: ownerCookie, origin: tlsEntry.url }); sockets.push(cookieSocketA);
    const cookieSocketB = await openSocket(tlsEntry.url, deviceB.token, { ca: tls.cert, cookie: ownerCookie, origin: tlsEntry.url }); sockets.push(cookieSocketB);
    let ownerBrowserSocket;
    await check('mobile_real_handshake_and_authenticated_websockets', async () => {
      const system = await mobile.api('/api/system/info');
      assert(system.storage_generation && system.agent_data_generation);
      report.storage_generation = system.storage_generation;
      return { desktop_id: info.desktop_id, storage_generation: system.storage_generation, agent_data_generation: system.agent_data_generation, two_wss_upgrades: true };
    });

    const model = await modelFixture(); resources.push(model);
    const provider = unwrap(await ownerRequest('/api/providers', { method: 'POST', body: {
      platform: 'custom', name: 'Connectivity acceptance model', base_url: `${model.url}/v1`, auth_scheme: 'bearer', credentials: { api_keys: ['local-fixture-not-a-secret'] }, enabled: true,
      initial_model: { model: 'connectivity-fixture', enabled: true, capabilities: [{ task: 'chat', traits: [], protocol: 'openai.chat_text', connection_role: 'default', output_limit: 1024 }] },
    } }), 'provider create', 201);
    const preset = unwrap(await ownerRequest('/api/agent-presets/from-template/chat.minimal', { method: 'POST', body: { reuse_existing: false, display_name: 'Connectivity acceptance', model_route_refs: {}, chat_route_records: {}, model: { provider_id: provider.provider_id, model: 'connectivity-fixture' } } }));
    const session = await mobile.api('/api/agent-sessions', { body: { preset_id: preset.preset.preset_id, title: 'Isolated connectivity acceptance', model: { provider_id: provider.provider_id, model: 'connectivity-fixture' } }, headers: { 'Idempotency-Key': randomId() } });
    const sessionId = session.agent_session_id; assert(sessionId);
    await check('mobile_canonical_turn_stream_and_durable_history', async () => {
      const turn = await mobile.api(`/api/agent-sessions/${sessionId}/turns`, { body: { input: { content: INPUT }, idempotency_key: randomId() } });
      assert.equal(turn.agent_session_id, sessionId); assert(turn.message_id && turn.operation_id);
      await until('canonical response history', async () => {
        const history = await mobile.api(`/api/agent-sessions/${sessionId}/message-history?page_size=40&cursor=`);
        return JSON.stringify(history).includes(REPLY) && history;
      }, { timeoutMs: 60_000 });
      await until('real WSS model stream', () => socketB.messages
        .filter(message => message.name === 'message.stream' && message.data?.conversation_id === sessionId)
        .map(message => message.data?.data?.content ?? '').join('').includes(REPLY));
      await until('same-origin device WSS with owner cookie receives canonical stream', () => cookieSocketB.messages
        .filter(message => message.name === 'message.stream' && message.data?.conversation_id === sessionId)
        .map(message => message.data?.data?.content ?? '').join('').includes(REPLY));
      assert.equal(model.calls.length, 1);
      return { agent_session_id: sessionId, operation_id: turn.operation_id, model_calls: 1, durable_assistant_reply: true, streamed_reply_received_over_wss: true };
    });
    await check('device_revoke_closes_existing_ws_and_keeps_other_device', async () => {
      unwrap(await ownerRequest(`/api/connections/devices/${deviceA.device.device_id}`, { method: 'DELETE' }));
      await until('revoked native and owner-cookie H5 WSS close', () => socketA.closed && cookieSocketA.closed, { timeoutMs: 10_000 });
      const dead = await publicRequest('/api/system/info', { headers: bearer(deviceA.token) });
      const deadRefresh = await publicRequest('/api/auth/refresh', { method: 'POST', headers: bearer(deviceA.token), body: { token: deviceA.token } });
      assert([401, 403].includes(dead.status)); assert([401, 403].includes(deadRefresh.status));
      const reconnect = await assertSocketAuthenticationRejected(tlsEntry.url, deviceA.token, { ca: tls.cert });
      const alive = await mobile.api('/api/system/info'); assert.equal(alive.storage_generation, report.storage_generation); assert.equal(socketB.closed, false); assert.equal(cookieSocketB.closed, false);
      return { revoked_http_status: dead.status, revoked_refresh_status: deadRefresh.status, revoked_socket_close_code: socketA.closeCode, revoked_reconnect: reconnect, other_device_http_and_ws_alive: true, owner_cookie_does_not_keep_revoked_device_ws_alive: true };
    });
    await check('explicit_device_ws_cannot_fall_back_to_owner_cookie_or_bearer', async () => {
      const revoked = await assertSocketAuthenticationRejected(tlsEntry.url, deviceA.token, { ca: tls.cert, cookie: ownerCookie, origin: tlsEntry.url });
      const invalidCookie = await assertSocketAuthenticationRejected(tlsEntry.url, 'invalid.payload.signature', { ca: tls.cert, cookie: ownerCookie, origin: tlsEntry.url });
      const invalidBearer = await assertSocketAuthenticationRejected(tlsEntry.url, 'header..signature', { ca: tls.cert, headers: bearer(ownerToken), cookie: ownerCookie, origin: tlsEntry.url });
      ownerBrowserSocket = await openSocket(tlsEntry.url, null, { ca: tls.cert, cookie: ownerCookie, origin: tlsEntry.url }); sockets.push(ownerBrowserSocket);
      assert.equal(ownerBrowserSocket.closed, false); assert.equal(cookieSocketB.closed, false);
      return { revoked_explicit_device_rejected_with_owner_cookie: revoked, invalid_explicit_device_rejected_with_owner_cookie: invalidCookie, invalid_explicit_device_rejected_with_owner_bearer: invalidBearer, pure_owner_cookie_browser_ws_remains_supported: true, other_device_ws_alive: true };
    });
    await check('uncertain_post_is_not_automatically_replayed', async () => {
      const lost = await startProxy({ target: tlsEntry.url, upstreamCa: tls.cert, dropResponse: incoming => incoming.method === 'POST' && incoming.url.endsWith('/turns') }); resources.push(lost);
      const lossyMobile = await createMobileClient({ transportModule, connectionModule, contractModule, baseUrl: lost.url, device: deviceB, info, ca: tls.cert });
      const before = model.calls.length;
      await assert.rejects(lossyMobile.api(`/api/agent-sessions/${sessionId}/turns`, { body: { input: { content: 'lost acknowledgement acceptance' }, idempotency_key: randomId() } }));
      await until('uncertain turn settles', async () => {
        const history = await mobile.api(`/api/agent-sessions/${sessionId}/message-history?page_size=40&cursor=`);
        return history.items?.filter(message => JSON.stringify(message.content).includes('lost acknowledgement acceptance')).length === 1 && model.calls.length === before + 1;
      }, { timeoutMs: 60_000 });
      await delay(500);
      assert.equal(lost.counters.get(`POST /api/agent-sessions/${sessionId}/turns`), 1);
      assert.equal(model.calls.length, before + 1);
      await until('pure owner-cookie browser receives a real canonical stream', () => ownerBrowserSocket.messages
        .filter(message => message.name === 'message.stream' && message.data?.conversation_id === sessionId)
        .map(message => message.data?.data?.content ?? '').join('').includes(REPLY));
      assert.equal(ownerBrowserSocket.closed, false);
      return { acknowledged_response_dropped_after_real_server_mutation: true, mutation_requests: 1, model_calls: 1, durable_user_rows: 1, pure_owner_cookie_browser_received_canonical_business_stream: true };
    });
    await check('relay_agent_disconnect_and_same_credentials_recovery', async () => {
      const stopStarted = Date.now(), stoppedPid = agent.child.pid;
      await processes.stop(agent);
      assert(agent.child.exitCode !== null || agent.child.signalCode !== null, 'Owned nfagent process must actually terminate');
      const stoppedExitCode = agent.child.exitCode, stoppedSignal = agent.child.signalCode;
      // Windows terminates the process without a QUIC close. The production
      // carrier permits 30 s idle. quic-go starts from the later first
      // ACK-eliciting send, so allow its 10 s keepalive plus 5 s scheduling.
      // Then separately bound downstream WS closure after the offline fact.
      const controlBefore = await until('Relay detects stopped carrier within its declared 30s idle plus 10s keepalive and 5s scheduling budgets', async () => {
        const current = unwrap(await request(bootstrap.control_url, { headers: bearer(bootstrap.control_token) }));
        return current.state !== 'transport_ready' && current;
      }, { timeoutMs: 45_000, intervalMs: 700 });
      const detectedAt = Date.now();
      await until('WSS tunnel closes after Relay reports carrier offline', () => socketB.closed, { timeoutMs: 3_000 });
      const websocketClosedAt = Date.now();
      const unavailable = await publicRequest('/api/system/info', { headers: bearer(deviceB.token), timeoutMs: 3_000 }).catch(() => ({ status: 0 }));
      assert(unavailable.status === 0 || unavailable.status >= 400, 'Disconnected relay must not claim success');
      assert.notEqual(controlBefore.state, 'transport_ready');
      agent = launch('agent-recovery', config.agentBin, agentArgs);
      await until('reconnected agent tunnel', async () => (await publicRequest('/api/system/info', { headers: bearer(deviceB.token) })).status === 200, { timeoutMs: 45_000 });
      const recovered = await openSocket(tlsEntry.url, deviceB.token, { ca: tls.cert }); sockets.push(recovered);
      const history = await mobile.api(`/api/agent-sessions/${sessionId}/message-history?page_size=40&cursor=`);
      assert(JSON.stringify(history).includes(REPLY));
      return { unavailable_http_status: unavailable.status, relay_state_while_disconnected: controlBefore.state, terminated_owned_agent_pid: stoppedPid,
        process_exit_code: stoppedExitCode, process_signal: stoppedSignal, carrier_detection_ms: detectedAt - stopStarted,
        websocket_closed_after_detection_ms: websocketClosedAt - detectedAt, carrier_idle_budget_ms: 30_000, first_ack_eliciting_send_allowance_ms: 10_000, scheduling_allowance_ms: 5_000,
        websocket_closure_budget_after_offline_ms: 3_000, same_agent_state_directory: true, new_wss_upgrade: true, canonical_history_survived: true };
    });

    await check('blocked_udp_falls_back_to_real_tls_tcp_carrier', async () => {
      await processes.stop(agent);
      const blocked = dgram.createSocket('udp4');
      let discarded = 0; blocked.on('message', () => { discarded++; });
      await new Promise((resolve, reject) => { blocked.once('error', reject); blocked.bind(0, '127.0.0.1', resolve); });
      resources.push({ close: () => new Promise(resolve => blocked.close(resolve)) });
      // This live sink drops QUIC packets. The actual Relay TLS carrier remains
      // reachable at its declared shared listener, using the same SPKI pin.
      agent = launch('agent-udp-blocked-l2', config.agentBin, ['-relay', `127.0.0.1:${blocked.address().port}`, '-pin', bootstrap.pin, '-token', bootstrap.enrol_token, '-state-dir', agentDirectory, '-l2-port', String(businessPort)]);
      await until('TLS/TCP fallback carries authenticated HTTP', async () => (await publicRequest('/api/system/info', { headers: bearer(deviceB.token) })).status === 200, { timeoutMs: 45_000 });
      const agents = unwrap(await relayAdmin('/api/agents'));
      const active = agents.find(item => item.online && item.rung === 'L2/TLS:443');
      assert(active && active.supports_datagram === false); assert(discarded > 0, 'QUIC must have attempted the blocked UDP path');
      const fallbackSocket = await openSocket(tlsEntry.url, deviceB.token, { ca: tls.cert }); sockets.push(fallbackSocket);
      const before = model.calls.length;
      const turn = await mobile.api(`/api/agent-sessions/${sessionId}/turns`, { body: { input: { content: 'real TLS TCP fallback turn' }, idempotency_key: randomId() } });
      assert.equal(turn.agent_session_id, sessionId);
      await until('model reply through real TLS/TCP and WSS', () => fallbackSocket.messages
        .filter(message => message.name === 'message.stream' && message.data?.conversation_id === sessionId)
        .map(message => message.data?.data?.content ?? '').join('').includes(REPLY), { timeoutMs: 60_000 });
      assert.equal(model.calls.length, before + 1);
      return { relay_reported_carrier: active.rung, supports_datagram: false, discarded_quic_packets: discarded, same_agent_identity: active.id, authenticated_http: true, real_wss_model_stream: true };
    });

    await check('target_identity_and_storage_generation_isolate_mobile', async () => {
      const secondPort = await freePort(), secondDirectory = join(fixture, 'desktop-second'); mkdirSync(secondDirectory);
      launch('desktop-second', config.desktopBin, ['--host', '127.0.0.1', '--port', String(secondPort), '--api-only', '--data-dir', secondDirectory], childEnvironment(secondDirectory, password));
      const secondUrl = `http://127.0.0.1:${secondPort}`;
      await until('second isolated Desktop', async () => (await request(`${secondUrl}/api/auth/status`)).status === 200, { timeoutMs: 90_000 });
      const secondLogin = unwrap(await request(`${secondUrl}/login`, { method: 'POST', body: { username: 'connectivity-owner', password } })); processes.secrets.add(secondLogin.token);
      const secondInfo = unwrap(await request(`${secondUrl}/api/connections/info`));
      const secondSystem = unwrap(await request(`${secondUrl}/api/system/info`, { headers: bearer(secondLogin.token) }));
      assert.notEqual(secondInfo.desktop_id, info.desktop_id); assert.notEqual(secondSystem.storage_generation, report.storage_generation);
      const isolated = await exerciseIsolation({ mobile, transportModule, connectionModule, contractModule, info, secondInfo, secondSystem, secondUrl, token: secondLogin.token, user: secondLogin.user, resources, businessUrl: tlsEntry.url, deviceB, tls, sessionId });
      return { distinct_desktop_identity: true, distinct_storage_generation: true, ...isolated };
    });
    await check('relay_connection_revoke_removes_resources', async () => {
      const revoked = unwrap(await request(bootstrap.control_url, { method: 'DELETE', headers: bearer(bootstrap.control_token) }));
      assert(revoked.revoked || revoked.state === 'revoked');
      const tunnels = unwrap(await relayAdmin('/api/tunnels'));
      const items = Array.isArray(tunnels) ? tunnels : tunnels.tunnels ?? tunnels.items;
      assert(Array.isArray(items), 'Relay tunnel list missing'); assert(!items.some(item => item.id === bootstrap.tunnel.id));
      return { connection_revoked: true, tunnel_removed: true };
    });
    await check('expired_bootstrap_can_be_cancelled_and_remains_revoked_after_relay_restart', () => expiredBootstrapAcceptance({ config, fixture, processes, allChildren, resources }));
    if (config.nativeHarnessBin) {
      await processes.stop(agent);
      await processes.stop(desktopProcess);
      await freePort('tcp', RESERVED_DESKTOP_PORT);
      report.native_managed_runtime = await nativeAcceptance({ config, root: ROOT, fixture, processes, allChildren, resources, check, relayAdmin });
      report.binaries.nativeHarnessBin = { path: config.nativeHarnessBin, sha256: createHash('sha256').update(readFileSync(config.nativeHarnessBin)).digest('hex') };
      report.boundaries = report.boundaries.map(value => value === 'Tauri shell/native lifecycle not measured' ? 'Tauri window UI not measured; production DesktopServer and RelayPairingManager exercised headlessly' : value);
    }
    report.result = 'pass';
  } catch (error) {
    report.result = 'fail'; report.error = processes.redact(error.message);
  } finally {
    try { await cleanup(); } catch (error) { report.result = 'fail'; report.cleanup_error = processes.redact(error.message); }
    report.finished_at = new Date().toISOString();
    writeFileSync(join(config.output, 'report.json'), `${JSON.stringify(report, null, 2)}\n`);
  }
  process.stdout.write(`${report.result.toUpperCase()} ${join(config.output, 'report.json')}\n`);
  return report;
}

// These two small adapters are intentionally aligned with the Mobile factory exports.
// The acceptance does not replace Mobile's transport or binding guards with a test client.
async function createMobileClient(options) {
  const { transportModule, connectionModule, contractModule, baseUrl, device, info, ca } = options;
  let persisted = null;
  const store = connectionModule.createConnectionStore({ read: async () => persisted, write: async value => { persisted = value; } });
  const desktop = contractModule.validateDesktopInfo(info);
  const system = contractModule.validateSystemInfo(unwrap(await request(`${baseUrl}/api/system/info`, { ca, headers: bearer(device.token) })));
  await store.connect({ baseUrl, token: device.token, user: device.user ?? null, device: device.device, desktop, system, contractVersion: desktop.protocol_version });
  const client = transportModule.createTransportClient({ binding: store.binding, epoch: store.epoch, markAuthExpired: store.markAuthExpired, fetch: fetchAdapter({ ca }), sameOrigin: false,
    verified: () => { const current = store.getState(); return current.phase === 'connected' && current.verified; } });
  return { ...client, store };
}

export async function importMobileModules(mobileRoot) {
  const { registerHooks } = await import('node:module');
  const prefix = pathToFileURL(`${resolve(mobileRoot, 'src/api')}/`).href;
  // Mobile's extensionless TypeScript imports are Metro conventions. Resolve
  // only real sibling files in this checkout, then let Node strip their types.
  const hooks = registerHooks({ resolve(specifier, context, next) {
    try { return next(specifier, context); }
    catch (error) {
      if (error.code !== 'ERR_MODULE_NOT_FOUND' || !context.parentURL?.startsWith(prefix) || !specifier.startsWith('./')) throw error;
      const candidate = new URL(`${specifier}.ts`, context.parentURL);
      if (!candidate.href.startsWith(prefix) || !existsSync(fileURLToPath(candidate))) throw error;
      return { url: candidate.href, shortCircuit: true };
    }
  } });
  try {
    const [transportModule, connectionModule, contractModule, pairingModule] = await Promise.all([
      import(pathToFileURL(join(mobileRoot, 'src/api/transport-client.ts')).href),
      import(pathToFileURL(join(mobileRoot, 'src/api/connection-core.ts')).href),
      import(pathToFileURL(join(mobileRoot, 'src/api/connection-contract.ts')).href),
      import(pathToFileURL(join(mobileRoot, 'src/api/pairing.ts')).href),
    ]);
    return { transportModule, connectionModule, contractModule, pairingModule };
  } finally { hooks.deregister(); }
}

async function exerciseIsolation(options) {
  const { transportModule, connectionModule, contractModule, info, secondInfo, secondSystem, secondUrl, token, user, resources, businessUrl, deviceB, tls, sessionId } = options;
  let release;
  const hold = new Promise(resolve => { release = resolve; });
  let reached = false;
  const delayed = await startProxy({ target: businessUrl, upstreamCa: tls.cert, holdResponse: incoming => {
    if (incoming.url === '/api/agent-sessions') { reached = true; return hold; }
    return undefined;
  } }); resources.push(delayed);
  const client = await createMobileClient({ transportModule, connectionModule, contractModule, baseUrl: delayed.url, device: deviceB, info, ca: tls.cert });
  const before = client.store.binding();
  const firstNamespace = contractModule.connectionNamespace(before);
  const pending = client.api('/api/agent-sessions').then(() => ({ accepted: true }), error => ({ accepted: false, code: error.code }));
  await until('delayed target A request reached real Desktop', () => reached);
  const desktop = contractModule.validateDesktopInfo(secondInfo), system = contractModule.validateSystemInfo(secondSystem);
  assert.throws(() => contractModule.assertDesktopIdentity(before.desktop, desktop), error => error.code === 'DESKTOP_IDENTITY_CHANGED');
  const differentGenerationNamespace = contractModule.connectionNamespace({ ...before, system });
  assert.notEqual(differentGenerationNamespace, firstNamespace, 'Cache namespace must include storage generation');
  await client.store.connect({ baseUrl: secondUrl, token, user, desktop, system });
  assert.notEqual(contractModule.connectionNamespace(client.store.binding()), firstNamespace);
  release();
  const result = await pending;
  assert.equal(result.accepted, false); assert.equal(result.code, 'CONNECTION_CHANGED');
  const secondSessions = await client.api('/api/agent-sessions');
  assert(!JSON.stringify(secondSessions).includes(sessionId), 'New Desktop must not retain old target business data');
  return { cache_namespace_includes_actual_storage_generation: true, late_old_target_response_rejected: true, old_session_absent_from_new_desktop: true, identity_mismatch_has_explicit_error: true };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { const config = parseArguments(process.argv.slice(2)); if (config.help) process.stdout.write(HELP); else process.exitCode = (await run(config)).result === 'pass' ? 0 : 1; }
  catch (error) { process.stderr.write(`Connectivity runner failed: ${error.message}\n`); process.exitCode = 1; }
}
