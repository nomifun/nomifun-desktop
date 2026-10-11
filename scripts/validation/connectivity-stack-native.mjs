import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { basename, join } from 'node:path';
import { delay, freePort, openSocket, request, startProxy, stdioRpc, until } from './connectivity-stack-helpers.mjs';
import { dedicatedRelay, shortTTLRelay } from './connectivity-stack-recovery.mjs';

const bearer = token => ({ Authorization: `Bearer ${token}` });
function unwrap(response) {
  assert.equal(response.status, 200, `Native acceptance API: HTTP ${response.status}`);
  assert(response.json && response.json.success !== false, 'Native acceptance API returned a product error');
  return response.json.data ?? response.json;
}

export function preloadManagedAgent(root, directory, binary) {
  const manifest = JSON.parse(readFileSync(join(root, 'apps/desktop/nfagent-runtime.json'), 'utf8'));
  const platform = { win32: 'windows', darwin: 'darwin', linux: 'linux' }[process.platform];
  const architecture = { x64: 'amd64', arm64: 'arm64' }[process.arch];
  const asset = manifest.assets[`${platform}-${architecture}`];
  assert(asset, 'Shipped managed-agent manifest does not support this platform');
  const contents = readFileSync(binary);
  const checksum = createHash('sha256').update(contents).digest('hex');
  assert.equal(checksum, asset.sha256.toLowerCase(), 'Managed agent must exactly match the shipped SHA-pinned manifest');
  const cache = join(directory, 'runtime', 'nfagent', manifest.version, checksum.slice(0, 16));
  mkdirSync(cache, { recursive: true });
  const destination = join(cache, basename(new URL(asset.url).pathname));
  writeFileSync(destination, contents, { mode: 0o700 });
  return { sha256: checksum, version: manifest.version };
}

export async function nativeAcceptance({ config, root, fixture, processes, allChildren, resources, check, relayAdmin }) {
  const directory = join(fixture, 'desktop-native'); mkdirSync(directory);
  writeFileSync(join(directory, '.nomifun-connection-acceptance'), 'connection-lifecycle-v1');
  let pinned;
  if (config.managedAgentBin) pinned = preloadManagedAgent(root, directory, config.managedAgentBin);
  const spa = config.nativeSpaDir ?? join(root, 'ui/dist');
  assert(existsSync(join(spa, 'index.html')), 'Native harness needs actual --native-spa-dir with built UI index.html');
  let rpc, processOwner, nativePhoneToken, nativePhoneSocket;
  const start = async label => {
    processOwner = processes.start(label, config.nativeHarnessBin, ['--data-dir', directory, '--spa-dir', spa], {
      NOMIFUN_DATA_DIR: directory, NOMIFUN_WORK_DIR: join(directory, 'work'), NOMIFUN_CACHE_DIR: join(directory, 'cache'), NOMIFUN_LOG_DIR: join(directory, 'logs'),
      NO_PROXY: '127.0.0.1,localhost', no_proxy: '127.0.0.1,localhost',
    }, { rpc: true });
    allChildren.push(processOwner);
    rpc = stdioRpc(processOwner, { secrets: processes.secrets });
    const boot = await rpc.call('boot');
    assert.equal(boot.relayPort, 25811);
    return boot;
  };
  const shutdown = async () => {
    if (!processOwner || processOwner.child.exitCode !== null || processOwner.child.signalCode !== null) return;
    await rpc.call('shutdown', {}, 20_000);
    await until('native harness owner exit', () => processOwner.child.exitCode !== null || processOwner.child.signalCode !== null, { timeoutMs: 15_000 });
  };
  resources.push({ close: shutdown });
  const firstBoot = await start('native-first');
  await check('native_owner_bootstrap_requires_local_trust', async () => {
    const origin = 'https://untrusted.example.test';
    const status = await request(`${firstBoot.backendUrl}/api/auth/status`, { headers: { Origin: origin } });
    assert.equal(status.status, 200);
    assert.equal(status.json.needs_setup, true);
    assert.equal(status.headers['access-control-allow-origin'], undefined);
    const info = await request(`${firstBoot.backendUrl}/api/connections/info`, { headers: { Origin: origin } });
    assert.equal(info.status, 200);
    assert.equal(info.headers['access-control-allow-origin'], undefined);
    const nativeHeaders = { Origin: 'http://tauri.localhost', 'x-nomi-local-trust': firstBoot.nativeTrust };
    assert(firstBoot.nativeTrust);
    assert.equal(unwrap(await request(`${firstBoot.backendUrl}/api/connections/devices`, { headers: nativeHeaders })).length, 0);
    const preflight = await request(`${firstBoot.backendUrl}/api/auth/device-setup`, { method: 'OPTIONS', headers: {
      Origin: origin, 'Access-Control-Request-Method': 'POST', 'Access-Control-Request-Headers': 'content-type',
    } });
    assert.equal(preflight.headers['access-control-allow-origin'], undefined);
    const password = 'Native-Owner-Acceptance-2026!';
    processes.secrets.add(password);
    for (const path of ['/api/auth/setup', '/api/auth/device-setup']) {
      const body = path.endsWith('device-setup')
        ? { username: 'native-owner', password, device_name: 'native-bootstrap', protocol_version: 1 }
        : { username: 'native-owner', password };
      // Both cross-origin browser calls and native HTTP calls must prove the
      // local owner authority. CORS alone cannot authorize initialization.
      for (const headers of [{ Origin: origin }, {}]) {
        const rejected = await request(`${firstBoot.backendUrl}${path}`, { method: 'POST', headers, body });
        assert.equal(rejected.status, 403, `${path} cannot initialize a native owner without local trust`);
        assert.equal(Boolean(rejected.json?.token || rejected.json?.device?.device_id), false);
        assert.equal(rejected.headers['access-control-allow-origin'], undefined);
      }
    }
    const unchanged = await request(`${firstBoot.backendUrl}/api/auth/status`);
    assert.equal(unchanged.json.needs_setup, true);
    assert.equal(unwrap(await request(`${firstBoot.backendUrl}/api/connections/devices`, { headers: nativeHeaders })).length, 0);
    const trusted = await request(`${firstBoot.backendUrl}/api/auth/device-setup`, { method: 'POST', headers: nativeHeaders,
      body: { username: 'native-owner', password, device_name: 'native-bootstrap', protocol_version: 1 } });
    assert.equal(trusted.status, 200);
    assert(trusted.json.token && trusted.json.device?.device_id);
    processes.secrets.add(trusted.json.token);
    assert.equal(trusted.headers['access-control-allow-origin'], nativeHeaders.Origin);
    assert.equal((await request(`${firstBoot.backendUrl}/api/auth/user`, { headers: bearer(trusted.json.token) })).status, 200);
    assert.equal((await request(`${firstBoot.backendUrl}/api/auth/status`)).json.needs_setup, false);
    return { arbitrary_origin_cors_denied: true, anonymous_http_setup_denied: true,
      both_setup_endpoints_denied: true, unauthorized_requests_preserve_owner_and_devices: true,
      legitimate_native_trust_setup_succeeds: true, device_credential_works: true };
  });
  const runtime = await check('native_shipped_runtime_is_sha_verified', async () => {
    const prepared = await rpc.call('prepare_runtime', {}, 320_000);
    const manifest = JSON.parse(readFileSync(join(root, 'apps/desktop/nfagent-runtime.json'), 'utf8'));
    const platform = { win32: 'windows', darwin: 'darwin', linux: 'linux' }[process.platform];
    const architecture = { x64: 'amd64', arm64: 'arm64' }[process.arch];
    const checksum = createHash('sha256').update(readFileSync(prepared.runtimePath)).digest('hex');
    assert.equal(checksum, manifest.assets[`${platform}-${architecture}`].sha256.toLowerCase());
    pinned = { sha256: checksum, version: manifest.version };
    return { ...pinned, runtime_owned_cache: prepared.runtimePath.startsWith(directory) };
  });
  assert(runtime.runtime_owned_cache);
  const businessPort = await freePort();
  const invite = unwrap(await relayAdmin('/api/pairing-invites/v2', { method: 'POST', body: { listen_port: businessPort, business_url: `http://127.0.0.1:${businessPort}`, note: 'native-managed-desktop' } }));
  processes.secrets.add(invite.token);
  const envelope = `nomifun-relay-pair:v2:${Buffer.from(JSON.stringify({ bootstrap_url: invite.bootstrap_url, invite: invite.token, expires_at: invite.expires_at })).toString('base64url')}`;
  processes.secrets.add(envelope);
  const status = await check('native_relay_only_connection_keeps_lan_disabled', async () => {
    const connected = await rpc.call('bootstrap', { envelope }, 120_000);
    assert.equal(connected.state, 'connected'); assert.equal(connected.enabled, true); assert(connected.agentPid);
    assert.equal(connected.webuiPort, 25811); assert(connected.health.backendReachable && connected.health.tunnelReachable);
    const lan = await rpc.call('lan_status'); assert.equal(lan.running, false);
    const anonymous = await request(`${connected.businessUrl}/api/system/info`); assert([401, 403].includes(anonymous.status));
    await assert.rejects(rpc.call('bootstrap', { envelope }, 30_000));
    const unchanged = await rpc.call('status');
    assert.equal(unchanged.state, 'connected'); assert.equal(unchanged.actionErrorCode, 'RELAY_ALREADY_PAIRED'); assert.equal(unchanged.tunnelId, connected.tunnelId);
    return { agent_pid: connected.agentPid, webui_port: connected.webuiPort, lan_disabled: true, public_requires_auth: true, second_enrollment_rejected_without_state_change: true, business_url: connected.businessUrl, tunnel_id: connected.tunnelId };
  });
  await check('native_consumed_pairing_qr_clears_and_explicit_refresh_is_fresh', async () => {
    const connected = await rpc.call('status');
    assert.equal(connected.qrState, 'absent'); assert.equal(connected.pairUrl ?? null, null);
    const ready = await rpc.call('refresh_qr');
    assert.equal(ready.qrState, 'ready'); assert(ready.pairUrl);
    const appUrl = new URL(ready.pairUrl), nested = new URL(appUrl.searchParams.get('url'));
    const qrToken = nested.searchParams.get('token'); assert(qrToken); processes.secrets.add(qrToken);
    const device = unwrap(await request(`${ready.businessUrl}/api/auth/device-pair`, { method: 'POST', body: { qr_token: qrToken, device_name: 'native-harness-phone', protocol_version: 1 } }));
    assert(device.device?.device_id && device.token); processes.secrets.add(device.token);
    nativePhoneToken = device.token;
    nativePhoneSocket = await openSocket(ready.businessUrl, nativePhoneToken);
    resources.push({ close: async () => nativePhoneSocket?.close() });
    const system = unwrap(await request(`${ready.businessUrl}/api/system/info`, { headers: bearer(device.token) })); assert(system.storage_generation);
    const consumed = await rpc.call('status'); assert.equal(consumed.qrState, 'used'); assert.equal(consumed.pairUrl, null);
    const fresh = await rpc.call('refresh_qr'); assert.equal(fresh.qrState, 'ready'); assert(fresh.pairUrl && fresh.pairUrl !== ready.pairUrl);
    const nextToken = new URL(new URL(fresh.pairUrl).searchParams.get('url')).searchParams.get('token'); assert.notEqual(nextToken, qrToken); processes.secrets.add(nextToken);
    return { device_credential_issued: true, actual_managed_agent_websocket_upgrade: true, consumed_qr_hidden: true, explicit_refresh_changes_ticket: true, no_code_without_explicit_request: true };
  });
  await check('native_pause_survives_process_restart', async () => {
    const started = Date.now();
    const paused = await rpc.call('pause'); assert.equal(paused.enabled, false); assert.equal(paused.state, 'paused'); assert.equal(paused.agentPid, null);
    await until('Relay detects paused managed carrier within its 30s idle plus 10s keepalive and 5s scheduling budgets', async () => {
      const connections = unwrap(await relayAdmin('/api/desktop-connections')).connections;
      const current = connections.find(connection => connection.tunnel_id === status.tunnel_id);
      return current && current.state !== 'transport_ready';
    }, { timeoutMs: 45_000, intervalMs: 700 });
    const detectedAt = Date.now();
    await until('paused managed carrier closes phone WSS after Relay offline fact', () => nativePhoneSocket.closed, { timeoutMs: 3_000 });
    await freePort('tcp', 25811);
    await shutdown();
    await start('native-after-paused-reboot');
    const restored = await rpc.call('status'); assert.equal(restored.state, 'paused'); assert.equal(restored.enabled, false); assert.equal(restored.agentPid, null);
    const lan = await rpc.call('lan_status'); assert.equal(lan.running, false); await freePort('tcp', 25811);
    return { paused_persisted: true, no_agent_after_reboot: true, relay_listener_closed: true, lan_disabled: true,
      carrier_detection_ms: detectedAt - started, carrier_detection_budget_ms: 45_000, websocket_closure_budget_after_offline_ms: 3_000 };
  });
  await check('native_restart_restores_existing_managed_agent_identity', async () => {
    const restarted = await rpc.call('restart', {}, 90_000);
    assert.equal(restarted.state, 'connected'); assert.equal(restarted.enabled, true); assert(restarted.agentPid);
    assert.equal(restarted.tunnelId, status.tunnel_id);
    const lan = await rpc.call('lan_status'); assert.equal(lan.running, false);
    const phoneSystem = unwrap(await request(`${restarted.businessUrl}/api/system/info`, { headers: bearer(nativePhoneToken) })); assert(phoneSystem.storage_generation);
    nativePhoneSocket = await openSocket(restarted.businessUrl, nativePhoneToken);
    return { same_tunnel: true, managed_agent_running: true, backend_and_tunnel_healthy: restarted.health.backendReachable && restarted.health.tunnelReachable, original_phone_http_and_websocket_recovered: true, lan_disabled: true };
  });
  await check('native_disconnect_revokes_relay_and_clears_local_state', async () => {
    const disconnected = await rpc.call('disconnect'); assert.equal(disconnected.state, 'disconnected'); assert.equal(disconnected.enabled, false); assert.equal(disconnected.agentPid, null);
    await freePort('tcp', 25811);
    const response = unwrap(await relayAdmin('/api/tunnels'));
    const tunnels = Array.isArray(response) ? response : response.tunnels ?? response.items;
    assert(Array.isArray(tunnels)); assert(!tunnels.some(tunnel => tunnel.id === status.tunnel_id));
    for (const file of ['relay-pairing.json', 'relay-agent']) assert(!existsSync(join(directory, file)), `Native disconnect left ${file}`);
    return { remote_tunnel_removed: true, local_pairing_and_agent_identity_removed: true, dedicated_listener_closed: true };
  });
  await check('native_unknown_bootstrap_expiry_survives_reboot_and_can_be_cancelled', async () => {
    const relay = await shortTTLRelay({ config, fixture, processes, allChildren, resources, name: 'relay-expiry-native' });
    const pendingInvite = await relay.invite('native-lost-bootstrap');
    const lost = await startProxy({ target: relay.baseUrl, dropResponse: (incoming, response) => incoming.method === 'POST' && incoming.url === '/api/bootstrap/desktop/v2' && response.statusCode === 200 });
    resources.push(lost);
    const pendingEnvelope = `nomifun-relay-pair:v2:${Buffer.from(JSON.stringify({ bootstrap_url: `${lost.url}/api/bootstrap/desktop/v2`, invite: pendingInvite.token, expires_at: pendingInvite.expires_at })).toString('base64url')}`;
    processes.secrets.add(pendingEnvelope);
    await assert.rejects(rpc.call('bootstrap', { envelope: pendingEnvelope }, 60_000));
    const pending = await rpc.call('status'); assert.equal(pending.pendingBootstrap, true);
    await freePort('tcp', 25811);
    const before = unwrap(await relay.admin('/api/desktop-connections')).connections;
    assert.equal(before.length, 1); assert(before[0].tunnel_id && before[0].agent_id);
    // The dev TTL is 2 s, with an inclusive integer expiry second. The lost
    // receipt has no accessible deadline, so cross that full second as well.
    await delay(3_200);
    await shutdown();
    await start('native-after-unknown-bootstrap-reboot');
    await until('native restore retains the expired uncertain bootstrap', async () => {
      const current = await rpc.call('status'); return current.pendingBootstrap && current.state === 'error' && current.actionErrorCode === 'RELAY_ENROL_EXPIRED';
    }, { timeoutMs: 30_000 });
    const restored = unwrap(await relay.admin('/api/desktop-connections')).connections;
    assert.equal(restored.length, 1); assert.equal(restored[0].tunnel_id, before[0].tunnel_id); assert.equal(restored[0].agent_id, before[0].agent_id);
    const cleared = await rpc.call('disconnect'); assert.equal(cleared.pendingBootstrap, false); assert.equal(cleared.state, 'disconnected');
    const tunnels = unwrap(await relay.admin('/api/tunnels')); assert(!tunnels.some(item => item.id === before[0].tunnel_id));
    const after = unwrap(await relay.admin('/api/desktop-connections')).connections; assert.equal(after[0].revoked, true);
    await freePort('tcp', 25811);
    return { real_bootstrap_success_response_dropped: true, pending_intent_persisted: true, idle_relay_edge_released: true, expired_receipt_recovery_failed_closed: true, no_duplicate_agent_or_tunnel: true, original_attempt_cancelled_by_production_manager: true, remote_tunnel_removed: true, dedicated_listener_closed: true };
  });
  await check('native_unreachable_relay_control_bounds_shutdown_and_offers_local_forget', async () => {
    const relay = await dedicatedRelay({ config, fixture, processes, allChildren, resources, name: 'relay-unreachable-control-native', enrolTtl: null, proxyBootstrap: true });
    const invite = await relay.invite('native-unreachable-control');
    assert(invite.bootstrap_url.startsWith(relay.bootstrapProxy.url), 'Relay control plane must be reachable only through the stoppable proxy');
    const controlEnvelope = `nomifun-relay-pair:v2:${Buffer.from(JSON.stringify({ bootstrap_url: invite.bootstrap_url, invite: invite.token, expires_at: invite.expires_at })).toString('base64url')}`;
    processes.secrets.add(controlEnvelope);
    const connected = await rpc.call('bootstrap', { envelope: controlEnvelope }, 120_000); assert.equal(connected.state, 'connected');
    const persisted = JSON.parse(readFileSync(join(directory, 'relay-pairing.json'), 'utf8'));
    assert(persisted.control_url.startsWith(relay.bootstrapProxy.url));
    await relay.bootstrapProxy.close();
    await shutdown();
    await start('native-unreachable-control-reboot');
    await delay(1_000);
    const shutdownStarted = Date.now(); await shutdown(); const shutdownMs = Date.now() - shutdownStarted;
    assert(shutdownMs < 15_000, `Shutdown waited ${shutdownMs} ms behind an unreachable Relay`);
    await start('native-unreachable-control-second-reboot');
    await delay(1_000);
    const disconnectStarted = Date.now();
    await assert.rejects(rpc.call('disconnect', {}, 30_000));
    const disconnectMs = Date.now() - disconnectStarted;
    assert(disconnectMs < 15_000, `Disconnect waited ${disconnectMs} ms behind restore`);
    const retained = await rpc.call('status');
    assert.equal(retained.actionErrorCode, 'RELAY_REVOKE_UNCONFIRMED'); assert(retained.relay);
    for (const file of ['relay-pairing.json', 'relay-agent', 'relay-control.secret']) assert(existsSync(join(directory, file)), `Unconfirmed disconnect removed ${file}`);
    const forgotten = await rpc.call('forget_local');
    assert.equal(forgotten.state, 'disconnected'); assert.equal(forgotten.agentPid, null); assert.equal(forgotten.relay ?? null, null);
    for (const file of ['relay-pairing.json', 'relay-agent', 'relay-control.secret', 'relay-bootstrap.pending.secret']) assert(!existsSync(join(directory, file)), `Local forget left ${file}`);
    await freePort('tcp', 25811);
    const remote = unwrap(await relay.admin('/api/desktop-connections')).connections;
    assert(remote.some(connection => connection.invite_id === persisted.invite_id && !connection.revoked), 'Local forget must not claim Relay-side revocation');
    return { shutdown_ms: shutdownMs, shutdown_budget_ms: 15_000, disconnect_ms: disconnectMs, superseded_restore_wait: true, unconfirmed_revoke_retained_local_state: true, local_forget_removed_state: true, relay_side_connection_left_for_console: true, dedicated_listener_closed: true };
  });
  await check('native_unreadable_relay_state_does_not_block_startup_and_can_be_forgotten', async () => {
    await shutdown();
    writeFileSync(join(directory, 'relay-bootstrap.pending.secret'), 'not-a-protected-relay-secret');
    await start('native-unreadable-relay-state');
    const unreadable = await rpc.call('status');
    assert.equal(unreadable.state, 'error'); assert.equal(unreadable.errorCode, 'RELAY_STATE_UNREADABLE'); assert.equal(unreadable.agentPid, null);
    const lan = await rpc.call('lan_status'); assert.equal(lan.running, false);
    await assert.rejects(rpc.call('restart', {}, 20_000));
    const forgotten = await rpc.call('forget_local');
    assert.equal(forgotten.state, 'disconnected'); assert.equal(forgotten.errorCode ?? null, null);
    assert(!existsSync(join(directory, 'relay-bootstrap.pending.secret')));
    await freePort('tcp', 25811);
    return { desktop_started_with_unreadable_relay_state: true, typed_state_error: true, unreadable_state_refuses_reconnect: true, local_forget_cleared_state: true };
  });
  await shutdown();
  return pinned;
}
