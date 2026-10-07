import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, readFile, readdir, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve, dirname, basename } from 'node:path';
import { parseArgs, decodeAudio, resampleMono, encodePacket, decodePacket, percentile, safeEvent, runSmoke } from './run-agent-voice-smoke.mjs';

const token = 'private-fixture-app-token';
const args = ['--run-live', '--base-url', 'http://127.0.0.1:9999', '--agent-session-id', 'fixture-session', '--binding-version', '1', '--provider-id', 'fixture-provider', '--model', 'fixture-model', '--audio', 'fixture.pcm', '--source-identity', 'f'.repeat(40)];
const format = { encoding: 'pcm', sample_rate: 16000, channels: 1, sample_format: 'signed16_le' };
test('live calls require explicit opt-in, exact identities, bounds and credentials', () => {
  assert.throws(() => parseArgs(args.slice(1), {}), /LIVE_OPT_IN_REQUIRED/);
  assert.throws(() => parseArgs(args, {}), /AUTH_TOKEN_REQUIRED/);
  assert.throws(() => parseArgs([...args, '--duration', '121'], { NOMIFUN_VOICE_SMOKE_AUTH_TOKEN: token }), /INVALID_DURATION/);
  assert.throws(() => parseArgs([...args, '--api-key', token], { NOMIFUN_VOICE_SMOKE_AUTH_TOKEN: token }), /INVALID_ARGUMENT/);
  const remote = [...args]; remote[2] = 'http://remote.example'; assert.throws(() => parseArgs(remote, { NOMIFUN_VOICE_SMOKE_AUTH_TOKEN: token }), /REMOTE_HTTP_AUTH_FORBIDDEN/);
  assert.equal(parseArgs(args, { NOMIFUN_VOICE_SMOKE_AUTH_TOKEN: token }).model, 'fixture-model');
});
test('raw audio cannot infer rate/channels and partial sample frames fail', () => {
  assert.throws(() => decodeAudio(Buffer.alloc(4)), /RAW_AUDIO_SPEC_REQUIRED/);
  assert.throws(() => decodeAudio(Buffer.alloc(3), 16000, 1), /PARTIAL_PCM_SAMPLE_FRAME/);
  const stereo = Buffer.alloc(4800 * 2 * 2); for (let i = 0; i < stereo.length; i += 2) stereo.writeInt16LE(1000, i);
  const converted = resampleMono(decodeAudio(stereo, 48000, 2), 16000); assert.equal(converted.length, 3200); assert.equal(converted.readInt16LE(0), 1000);
});
test('NFV1 preserves format/duration and rejects old or malformed frames', () => {
  const metadata = { segment_id: 's', activation_epoch: 1, output_generation: 2, sequence: 1, timestamp: 0, duration_us: 20000, format };
  const packet = encodePacket(metadata, Buffer.alloc(640)); assert.deepEqual(decodePacket(packet).metadata, metadata);
  assert.throws(() => decodePacket(Buffer.from('old')), /MEDIA_PACKET_VERSION/);
  assert.throws(() => decodePacket(encodePacket({ ...metadata, duration_us: 30000 }, Buffer.alloc(640))), /INVALID_PCM_DURATION/);
});
test('telemetry omits speech text, tool args, summaries and diagnostics', () => {
  const input = { kind: 'model', event: { kind: 'work_trigger', trigger: { kind: 'typed_tool_call', upstream_trigger_id: 't', arguments: { action: 'steer', text: token } } } };
  assert.equal(safeEvent(input).action, 'steer'); assert.ok(!JSON.stringify(safeEvent(input)).includes(token));
  assert.ok(!JSON.stringify(safeEvent({ kind: 'work_receipt', receipt: { summary: token, status: 'queued' } })).includes(token));
  assert.ok(!JSON.stringify(safeEvent({ kind: 'model', event: { kind: 'error', error: { kind: 'network', message: token } } })).includes(token));
});
test('p50/p95 absent evidence stays null', () => { assert.equal(percentile([], 0.95), null); assert.equal(percentile([1, 4, 2, 3], 0.5), 2); assert.equal(percentile([1, 4, 2, 3], 0.95), 4); });

test('runner observes local fixture honestly, redacts secrets and only closes voice', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'nomifun-voice-smoke-runner-'));
  try {
    const audio = join(directory, 'fixture.pcm'); await writeFile(audio, Buffer.alloc(640));
    const config = parseArgs([...args.slice(0, -4), '--audio', audio, '--source-identity', 'f'.repeat(40), '--sample-rate', '16000', '--channels', '1', '--duration', '1', '--output', directory], { NOMIFUN_VOICE_SMOKE_AUTH_TOKEN: token });
    const calls = []; let clock = 0, socket;
    class FixtureSocket {
      constructor(url, protocols) { assert.ok(!url.includes('fixture-attachment-secret')); assert.deepEqual(protocols,['nomifun.voice.v1','nomifun.voice.ticket.fixture-attachment-secret']); this.readyState = 1; this.bufferedAmount = 0; socket = this; queueMicrotask(() => {this.onopen();this.onmessage({data:JSON.stringify({kind:'state',state:{voice_session_id:'fixture-voice',activation_epoch:1,output_generation:1,connection:'ready',capture:'capturing'}})});}); }
      send(packet) {
        if (typeof packet === 'string') { assert.equal(JSON.parse(packet).kind, 'foreground'); return; }
        const uplink=decodePacket(packet).metadata; this.onmessage({data:JSON.stringify({kind:'input_admitted',activation_epoch:uplink.activation_epoch,sequence:uplink.sequence,duration_us:uplink.duration_us})});
        assert.equal(Buffer.from(packet).subarray(0, 4).toString(), 'NFV1');
        if (this.emitted) return; this.emitted = true;
        this.onmessage({ data: JSON.stringify({ kind: 'model', event: { kind: 'work_trigger', trigger: { kind: 'typed_tool_call', upstream_trigger_id: 't', arguments: { action: 'start', text: token } } } }) });
        this.onmessage({ data: JSON.stringify({ kind: 'work_receipt', upstream_trigger_id: 't', receipt: { receipt_id: token, operation_key: 'o', status: 'queued', summary: token } }) });
        this.onmessage({ data: encodePacket({ segment_id: 's', activation_epoch: 1, output_generation: 1, sequence: 1, timestamp: 0, duration_us: 20000, format }, Buffer.alloc(640)) });
      }
      close() { this.readyState = 3; this.onclose?.(); }
    }
    const fetch = async (url, options) => {
      calls.push({ path: new URL(url).pathname, body: options.body ? JSON.parse(options.body) : null });
      let data;
      if (url.includes('/catalog')) data = { models: [{ provider_id: config.providerId, model: config.model, adapters: [{support: 'supported', descriptor: {transports: ['relay']}}] }] };
      else if (url.includes('/availability')) data = { enabled: true, binding_version: 1, profile: {profile_id: 'fixture-profile', revision: 1, route: {provider_id: config.providerId, model: config.model, route_id: 'fixture-route', revision:1}}, transport: 'relay' };
      else if (url.endsWith('/control')) { socket.onmessage({ data: JSON.stringify({ kind: 'model', event: { kind: 'closed', termination: { reason: 'user_ended', finalization_confirmed: true, message: token } } }) }); data = { connection: 'closed' }; }
      else data = { voice_session_id: 'fixture-voice', activation_epoch: 1, attachment_token: 'fixture-attachment-secret', negotiation: { input_spec: { format, min_frame_duration_us: 10000, max_frame_duration_us: 40000, max_frame_bytes: 1280 } } };
      return { ok: true, json: async () => ({ success: true, data }) };
    };
    const report = await runSmoke(config, { fetch, WebSocket: FixtureSocket, now: () => clock, sleep: async ms => { clock += ms; }, evidenceSource: 'local_fixture' });
    assert.equal(report.implementation_result, 'local_runner_test_passed'); assert.equal(report.audio_experience_result, 'pending');
    assert.equal(report.metrics.admission_ms.samples, 0, 'queued is not formal work admission');
    assert.equal(report.metrics.local_audible_stop_ms.p95, null); assert.equal(report.metrics.playback_underflows, null);
    assert.ok(!JSON.stringify(report).includes(token));
    assert.deepEqual(calls.filter(call => call.path.endsWith('/control')).map(call => call.body.control), [{ kind: 'close', reason: 'user_ended' }]);
    for (const file of (await readdir(directory)).filter(file => file.endsWith('.json') || file.endsWith('.jsonl'))) { const text = await readFile(join(directory, file), 'utf8'); assert.ok(!text.includes(token)); assert.ok(!text.includes('fixture-attachment-secret')); }
  } finally {
    const target = resolve(directory);
    assert.equal(dirname(target), resolve(tmpdir()), 'cleanup stays inside the task temporary directory');
    assert.ok(basename(target).startsWith('nomifun-voice-smoke-runner-'));
    await rm(target, { recursive: true, force: true });
  }
});

async function recoveryFixture(mode = 'healthy') {
  const directory = await mkdtemp(join(tmpdir(), 'nomifun-voice-recovery-runner-'));
  const audio = join(directory, 'fixture.pcm'); await writeFile(audio, Buffer.alloc(6400));
  const config = parseArgs([...args.slice(0, -4), '--audio', audio, '--source-identity', 'f'.repeat(40), '--sample-rate', '16000', '--channels', '1', '--duration', mode === 'end_pending' ? '1.5' : '3', '--interrupt-at-ms', '60', '--output', directory], { NOMIFUN_VOICE_SMOKE_AUTH_TOKEN: token });
  let clock = 0, socket, recoveryDue = null, recoverySent = false, secondSent = false, pauseSent = false;
  const packets = [], controls = [], heartbeats = [], pending = [], states = [];
  const state = (generation = 1, connection = 'ready', capture = 'capturing') => ({ voice_session_id: 'fixture-voice', activation_epoch: 1, output_generation: generation, connection, capture });
  const emit = product => socket.onmessage({ data: JSON.stringify(product) });
  const changed = value => { states.push(value); emit({ kind: 'state', state: value }); };
  const model = event => emit({ kind: 'model', event });
  const pump = () => {
    if (recoveryDue !== null && !recoverySent && clock >= recoveryDue) { recoverySent = true; changed(state(2, 'ready', 'paused')); model({ kind: 'relay_recovered', output_generation: 2 }); }
    if (mode === 'user_pause' && !pauseSent && clock >= 1700) { pauseSent = true; model({ kind: 'input_mute_applied', request_id: 'new-user-pause', muted: true }); changed(state(2, 'ready', 'paused')); }
    if (mode === 'new_recovery' && !secondSent && clock >= 1800) { secondSent = true; changed(state(3, 'recovering', 'paused')); model({ kind: 'relay_recovering', output_generation: 3 }); }
    if (mode === 'new_recovery' && secondSent && clock >= 2400 && !states.some(value => value.output_generation === 3 && value.connection === 'ready')) { changed(state(3, 'ready', 'paused')); model({ kind: 'relay_recovered', output_generation: 3 }); }
    for (const item of pending.filter(item => !item.done && clock >= item.due)) {
      item.done = true;
      const value = state(item.generation);
      if (mode !== 'user_pause' && mode !== 'wrong_epoch' && !(mode === 'new_recovery' && item.generation === 2) && mode !== 'end_pending') changed(value);
      item.resolve({ ok: true, json: async () => ({ success: true, data: mode === 'wrong_epoch' ? {...value,activation_epoch:2} : value }) });
    }
  };
  class FixtureSocket {
    constructor() { this.readyState = 1; this.bufferedAmount = 0; socket = this; queueMicrotask(() => { this.onopen(); changed(state()); }); }
    send(packet) {
      if (typeof packet === 'string') { heartbeats.push({ at: clock, payload: JSON.parse(packet) }); return; }
      const frame = decodePacket(packet).metadata; packets.push({ at: clock, frame });
      if (mode === 'released_only') emit({ kind: 'input_released', activation_epoch: 1, sequence: frame.sequence, duration_us: frame.duration_us });
      else if (mode !== 'no_credit' && frame.sequence !== 3) emit({ kind: 'input_admitted', activation_epoch: 1, sequence: frame.sequence, duration_us: frame.duration_us });
      if (packets.length === 1) this.onmessage({ data: encodePacket({ segment_id: 's', activation_epoch: 1, output_generation: 1, sequence: 1, timestamp: 0, duration_us: 20000, format }, Buffer.alloc(640)) });
    }
    close() { this.readyState = 3; this.onclose?.(); }
  }
  const fetch = async (url, options) => {
    if (url.includes('/catalog')) return { ok: true, json: async () => ({ success: true, data: { models: [{ provider_id: config.providerId, model: config.model, adapters: [{ support: 'supported', descriptor: { transports: ['relay'] } }] }] } }) };
    if (url.includes('/availability')) return { ok: true, json: async () => ({ success: true, data: { enabled: true, binding_version: 1, profile: { profile_id: 'fixture-profile', revision: 1, route: { provider_id: config.providerId, model: config.model, route_id: 'fixture-route', revision: 1 } }, transport: 'relay' } }) };
    if (url.endsWith('/control')) {
      const control = JSON.parse(options.body).control; controls.push({ at: clock, control, signal: options.signal });
      if (control.kind === 'close') { model({ kind: 'closed', termination: { reason: 'user_ended', finalization_confirmed: true } }); return { ok: true, json: async () => ({ success: true, data: state(2, 'closed', 'idle') }) }; }
      if (control.kind === 'interrupt_output') {
        if (mode !== 'no_credit' && mode !== 'released_only') {
          const tail = packets.find(packet => packet.frame.sequence === 3)?.frame; if (tail) emit({ kind: 'input_released', activation_epoch: 1, sequence: tail.sequence, duration_us: tail.duration_us });
          changed(state(2, 'recovering', 'paused')); model({ kind: 'relay_recovering', output_generation: 2 }); recoveryDue = clock + 1200;
        }
        return { ok: true, json: async () => ({ success: true, data: state(2, mode === 'no_credit' || mode === 'released_only' ? 'ready' : 'recovering', mode === 'no_credit' || mode === 'released_only' ? 'capturing' : 'paused') }) };
      }
      assert.deepEqual(control, { kind: 'mute_input', muted: false }); const generation = secondSent ? 3 : 2;
      return new Promise(resolveResponse => pending.push({ generation, due: clock + (generation === 3 ? 300 : 1100), resolve: resolveResponse, done: false }));
    }
    return { ok: true, json: async () => ({ success: true, data: { voice_session_id: 'fixture-voice', activation_epoch: 1, attachment_token: 'fixture-attachment-secret', negotiation: { input_spec: { format, min_frame_duration_us: 10000, max_frame_duration_us: 40000, max_frame_bytes: 1280 } } } }) };
  };
  try {
    const report = await runSmoke(config, { fetch, WebSocket: FixtureSocket, now: () => clock, sleep: async ms => { clock += ms; pump(); await Promise.resolve(); }, evidenceSource: 'local_fixture' });
    const sentAtClose = packets.length; clock += 5000; pump(); for (let i = 0; i < 8; i++) await Promise.resolve();
    assert.equal(packets.length, sentAtClose, 'late ACK after close cannot reopen capture');
    return { report, packets, controls, heartbeats, pending };
  } finally { await rm(directory, { recursive: true, force: true }); }
}

test('healthy relay recovery keeps heartbeats, returns released credit and resumes only after real state ACK', async () => {
  const result = await recoveryFixture();
  assert.equal(result.report.implementation_result, 'local_runner_test_passed'); assert.equal(result.report.audio_experience_result, 'pending');
  assert.ok(result.packets.some(packet => packet.at > 2400));
  assert.ok(result.packets.every(packet => packet.at < 60 || packet.at >= 2360), 'no uplink during opening or pending unmute ACK');
  assert.ok(result.heartbeats.some(heartbeat => heartbeat.at >= 1000 && heartbeat.at < 2000));
  assert.ok(result.heartbeats.some(heartbeat => heartbeat.at >= 2000));
  assert.equal(result.report.input_credit.released_frames, 1); assert.equal(result.report.input_credit.admitted_frames, result.report.input_credit.sent_frames - 1);
  assert.equal(result.controls.filter(item => item.control.kind === 'mute_input').length, 1);
  assert.ok(result.controls.every(item => ['mute_input', 'interrupt_output', 'close'].includes(item.control.kind)));
});
test('new pause or newer recovery invalidates a late unmute ACK', async () => {
  const paused = await recoveryFixture('user_pause'); assert.equal(paused.report.implementation_result, 'local_runner_test_passed');
  assert.ok(paused.packets.every(packet => packet.at < 60));
  const newer = await recoveryFixture('new_recovery'); assert.equal(newer.report.implementation_result, 'local_runner_test_passed');
  assert.ok(newer.packets.every(packet => packet.at < 60 || packet.frame.output_generation === 3));
  assert.equal(newer.controls.filter(item => item.control.kind === 'mute_input').length, 2);
});
test('released credits permit progress without being counted as model admission, missing credit fails honestly', async () => {
  const released = await recoveryFixture('released_only'); assert.equal(released.report.implementation_result, 'local_runner_test_passed');
  assert.equal(released.report.input_credit.admitted_frames, 0); assert.equal(released.report.input_credit.released_frames, released.report.input_credit.sent_frames);
  const missing = await recoveryFixture('no_credit'); assert.equal(missing.report.implementation_result, 'failed'); assert.equal(missing.report.failure_code, 'REMOTE_INPUT_CREDIT_EXHAUSTED');
});
test('ending while unmute is pending aborts its request and keeps late ACK inert', async () => {
  const result = await recoveryFixture('end_pending'); assert.equal(result.report.implementation_result, 'local_runner_test_passed');
  assert.ok(result.packets.every(packet => packet.at < 60)); assert.ok(result.controls.find(item => item.control.kind === 'mute_input').signal.aborted);
  assert.equal(result.controls.filter(item => item.control.kind === 'close').length, 1);
});
test('unmute result from another activation cannot confirm resumed input',async()=>{
  const result=await recoveryFixture('wrong_epoch');assert.equal(result.report.implementation_result,'failed');assert.equal(result.report.failure_code,'INPUT_RESUME_ACK_MISMATCH');
  assert.ok(result.packets.every(packet=>packet.at<60));
});

test('Live metadata correction latency uses executed request kind and keeps unknown work unclassified', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'nomifun-voice-neutral-metrics-'));
  try {
    const audio = join(directory, 'fixture.pcm'); await writeFile(audio, Buffer.alloc(640));
    const config = parseArgs([...args.slice(0, -4), '--audio', audio, '--source-identity', 'f'.repeat(40), '--sample-rate', '16000', '--channels', '1', '--duration', '1', '--expect-work', '--output', directory], { NOMIFUN_VOICE_SMOKE_AUTH_TOKEN: token });
    let clock = 0, socket, pending = false, applied = false;
    const controls = [];
    const emit = product => socket.onmessage({ data: JSON.stringify(product) });
    const receipt = (status, requestKind, upstreamId = null, operationKey = 'correction-operation') => emit({ kind: 'work_receipt', request_kind: requestKind, upstream_trigger_id: upstreamId, receipt: { receipt_id: `${operationKey}:${status}`, operation_key: operationKey, status, summary: token } });
    class FixtureSocket {
      constructor() { this.readyState = 1; this.bufferedAmount = 0; socket = this; queueMicrotask(() => { this.onopen(); emit({ kind: 'state', state: { voice_session_id: 'fixture-voice', activation_epoch: 1, output_generation: 1, connection: 'ready', capture: 'capturing' } }); }); }
      send(packet) {
        if (typeof packet === 'string') return;
        const frame = decodePacket(packet).metadata; emit({ kind: 'input_admitted', activation_epoch: 1, sequence: frame.sequence, duration_us: frame.duration_us });
        if (this.emitted) return; this.emitted = true;
        for (const upstreamId of ['live-delegation', 'unknown-delegation']) {
          const event = { kind: 'model', event: { kind: 'work_trigger', trigger: { kind: 'delegation', upstream_trigger_id: upstreamId, context_window_ref: 'live-context', intent: token } } };
          assert.equal(safeEvent(event).action, null, 'native metadata has no typed tool action'); emit(event);
        }
        this.onmessage({ data: encodePacket({ segment_id: 's', activation_epoch: 1, output_generation: 1, sequence: 1, timestamp: 0, duration_us: 20000, format }, Buffer.alloc(640)) });
      }
      close() { this.readyState = 3; this.onclose?.(); }
    }
    const fetch = async (url, options) => {
      let data;
      if (url.includes('/catalog')) data = { models: [{ provider_id: config.providerId, model: config.model, adapters: [{ support: 'supported', descriptor: { transports: ['relay'] } }] }] };
      else if (url.includes('/availability')) data = { enabled: true, binding_version: 1, profile: { profile_id: 'fixture-profile', revision: 1, route: { provider_id: config.providerId, model: config.model, route_id: 'fixture-route', revision: 1 } }, transport: 'relay' };
      else if (url.endsWith('/control')) { controls.push(JSON.parse(options.body).control); emit({ kind: 'model', event: { kind: 'closed', termination: { reason: 'user_ended', finalization_confirmed: true } } }); data = { connection: 'closed' }; }
      else data = { voice_session_id: 'fixture-voice', activation_epoch: 1, attachment_token: 'fixture-attachment-secret', negotiation: { input_spec: { format, min_frame_duration_us: 10000, max_frame_duration_us: 40000, max_frame_bytes: 1280 } } };
      return { ok: true, json: async () => ({ success: true, data }) };
    };
    const report = await runSmoke(config, { fetch, WebSocket: FixtureSocket, now: () => clock, sleep: async ms => {
      clock += ms;
      if (!pending && clock >= 40) { pending = true; receipt('pending_boundary', 'steer', 'live-delegation'); receipt('pending_boundary', undefined, 'unknown-delegation', 'unknown-operation'); }
      if (!applied && clock >= 120) { applied = true; receipt('applied', 'steer'); receipt('applied', undefined, null, 'unknown-operation'); }
    }, evidenceSource: 'local_fixture' });
    assert.equal(report.implementation_result, 'local_runner_test_passed'); assert.equal(report.audio_experience_result, 'pending');
    assert.deepEqual(report.metrics.correction_applied_ms, { samples: 1, p50: 120, p95: 120, measurement: 'work_trigger_received_to_canonical_applied', classification: 'application_executed_request_kind' });
    assert.equal(report.metrics.admission_ms.samples, 0);
    assert.equal(report.canonical_receipts.filter(value => value.request_kind === null).length, 2);
    assert.ok(!JSON.stringify(report).includes(token));
    assert.deepEqual(controls, [{ kind: 'close', reason: 'user_ended' }]);
  } finally { await rm(directory, { recursive: true, force: true }); }
});
