import assert from 'node:assert/strict';
import { randomBytes } from 'node:crypto';
import { mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { freePort, request, startProxy, until } from './connectivity-stack-helpers.mjs';

const bearer = token => ({ Authorization: `Bearer ${token}` });
function unwrap(result) {
  assert.equal(result.status, 200, `Expired-bootstrap API: HTTP ${result.status}`);
  assert(result.json && result.json.success !== false, 'Expired-bootstrap API rejected the operation');
  return result.json.data ?? result.json;
}

export async function dedicatedRelay({ config, fixture, processes, allChildren, resources, name, enrolTtl = '2s', proxyBootstrap = false }) {
  const directory = join(fixture, name); mkdirSync(directory);
  const udp = await freePort('udp'), consolePort = await freePort();
  const baseUrl = `http://127.0.0.1:${consolePort}`;
  let bootstrapProxy;
  if (proxyBootstrap) { bootstrapProxy = await startProxy({ target: baseUrl }); resources.push(bootstrapProxy); }
  const bootstrapUrl = `${bootstrapProxy?.url ?? baseUrl}/api/bootstrap/desktop/v2`;
  const args = ['-data-dir', directory, '-bind', `127.0.0.1:${udp}`, '-advertise', `127.0.0.1:${udp}`, '-console', `127.0.0.1:${consolePort}`,
    '-desktop-bootstrap-url', bootstrapUrl, '-desktop-dev-insecure', '-drain', '1s'];
  if (enrolTtl) args.push('-desktop-dev-enrol-ttl', enrolTtl);
  let child, cookies = '', csrf = '';
  const password = `Local-${randomBytes(32).toString('hex')}!`; processes.secrets.add(password);
  const start = async label => {
    child = processes.start(label, config.relayBin, args); allChildren.push(child);
    await until('short TTL real Relay startup', async () => (await request(`${baseUrl}/healthz`)).status === 200);
  };
  const login = async setup => {
    const result = await request(`${baseUrl}${setup ? '/api/setup' : '/api/login'}`, { method: 'POST', headers: { Origin: baseUrl }, body: { username: 'expiry-owner', password } });
    const parsed = unwrap(result); csrf = parsed.csrf_token;
    cookies = (result.headers['set-cookie'] ?? []).map(cookie => cookie.split(';')[0]).join('; ');
    assert(csrf && cookies); processes.secrets.add(csrf); processes.secrets.add(cookies);
  };
  await start(`${name}-first`); await login(true);
  const admin = (path, options = {}) => request(`${baseUrl}${path}`, { ...options, headers: { Cookie: cookies, Origin: baseUrl, 'X-NFR-CSRF': csrf, ...options.headers } });
  const stop = () => processes.stop(child);
  resources.push({ close: stop });
  return {
    baseUrl, admin, bootstrapProxy,
    invite: async note => {
      const port = await freePort();
      const invite = unwrap(await admin('/api/pairing-invites/v2', { method: 'POST', body: { listen_port: port, business_url: `http://127.0.0.1:${port}`, note } }));
      processes.secrets.add(invite.token); return invite;
    },
    restart: async () => { await stop(); await start(`${name}-restart`); await login(false); },
  };
}

export const shortTTLRelay = options => dedicatedRelay({ ...options, enrolTtl: '2s' });

export async function expiredBootstrapAcceptance(options) {
  const relay = await shortTTLRelay({ ...options, name: 'relay-expiry-api' });
  const invite = await relay.invite('expired-bootstrap-direct');
  const attempt = randomBytes(32).toString('hex');
  const first = unwrap(await request(invite.bootstrap_url, { method: 'POST', headers: bearer(invite.token), body: { attempt_id: attempt } }));
  for (const token of [first.enrol_token, first.control_token]) options.processes.secrets.add(token);
  const expiry = Date.parse(first.enrol_expires_at);
  assert(Number.isFinite(expiry) && expiry <= Date.now() + 3_000, 'Dev-only enrol TTL must bound this actual receipt');
  // Store time is integer Unix seconds and the expiry second is inclusive.
  // Cross its end, then issue one explicit receipt-recovery request.
  await until('actual declared enrol expiry second has ended', () => Date.now() >= expiry + 1_000, { timeoutMs: 5_000, intervalMs: 50 });
  const expired = await request(invite.bootstrap_url, { method: 'POST', headers: bearer(invite.token), body: { attempt_id: attempt } });
  assert.equal(expired.status, 410); assert.equal(expired.json?.error?.code ?? expired.json?.code, 'enrol_expired');
  const cancelUrl = `${relay.baseUrl}/api/bootstrap/desktop/v2:cancel`;
  const wrong = await request(cancelUrl, { method: 'POST', headers: bearer(invite.token), body: { attempt_id: randomBytes(32).toString('hex') } });
  assert.equal(wrong.status, 409);
  const cancelled = unwrap(await request(cancelUrl, { method: 'POST', headers: bearer(invite.token), body: { attempt_id: attempt } }));
  assert.equal(cancelled.revoked, true);
  const tunnels = unwrap(await relay.admin('/api/tunnels')); assert(!tunnels.some(item => item.id === first.tunnel.id));
  await relay.restart();
  const repeated = unwrap(await request(cancelUrl, { method: 'POST', headers: bearer(invite.token), body: { attempt_id: attempt } }));
  assert.equal(repeated.revoked, true); assert.equal(repeated.invite_id, cancelled.invite_id);
  return { actual_enrol_expiry_status: expired.status, mismatching_attempt_rejected: wrong.status, same_attempt_cancel_succeeds: true, remote_tunnel_removed: true, idempotent_after_real_relay_restart: true };
}
