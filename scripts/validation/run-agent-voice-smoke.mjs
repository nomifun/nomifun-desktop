#!/usr/bin/env bun
/** Opt-in, bounded product relay validation. Never launches a GUI or asserts audible playback. */
import { readFile, writeFile, mkdir, stat } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { randomUUID } from 'node:crypto';

const MAX_MEDIA_BYTES = 32 * 1024 * 1024;
const AUTH_ENV = 'NOMIFUN_VOICE_SMOKE_AUTH_TOKEN';
const CASES = new Set(['conversation', 'backchannel', 'correction', 'approval', 'long_work', 'network', 'device', 'agent_switch', 'takeover']);
const WORK_REQUEST_KINDS = new Set(['start', 'steer', 'observe', 'cancel', 'cancel_queued', 'revise_queued', 'answer_approval']);
export class SmokeError extends Error { constructor(code) { super(code); this.code = code; } }
const fail = code => { throw new SmokeError(code); };
export function parseArgs(argv, environment = process.env) {
  if (argv.includes('--help')) return { help: true };
  const flags = new Set(['--run-live', '--expect-work', '--capture-output']);
  const values = new Set(['--base-url', '--agent-session-id', '--binding-version', '--provider-id', '--model', '--audio', '--sample-rate', '--channels', '--duration', '--interrupt-at-ms', '--scenario', '--output', '--source-identity']);
  const args = {};
  for (let i = 0; i < argv.length; i += 1) {
    const key = argv[i];
    if (key in args) fail('DUPLICATE_ARGUMENT');
    if (flags.has(key)) args[key] = true;
    else if (values.has(key) && argv[i + 1] && !argv[i + 1].startsWith('--')) args[key] = argv[++i];
    else fail('INVALID_ARGUMENT');
  }
  if (!args['--run-live']) fail('LIVE_OPT_IN_REQUIRED');
  for (const key of ['--base-url', '--agent-session-id', '--binding-version', '--provider-id', '--model', '--audio', '--source-identity']) if (!args[key]) fail('REQUIRED_ARGUMENT_MISSING');
  let base;
  try { base = new URL(args['--base-url']); } catch { fail('INVALID_BASE_URL'); }
  if (!['http:', 'https:'].includes(base.protocol) || base.username || base.password || base.search || base.hash) fail('INVALID_BASE_URL');
  if (base.protocol === 'http:' && !['localhost', '127.0.0.1', '[::1]'].includes(base.hostname)) fail('REMOTE_HTTP_AUTH_FORBIDDEN');
  if (!/^[0-9a-f]{40,64}$/i.test(args['--source-identity'])) fail('SOURCE_IDENTITY_REQUIRED');
  const bindingVersion = Number(args['--binding-version']);
  const duration = Number(args['--duration'] ?? 30);
  const interruptAtMs = args['--interrupt-at-ms'] ? Number(args['--interrupt-at-ms']) : null;
  if (!Number.isSafeInteger(bindingVersion) || bindingVersion < 1 || !Number.isFinite(duration) || duration < 1 || duration > 120) fail('INVALID_DURATION_OR_BINDING');
  if (interruptAtMs !== null && (!Number.isFinite(interruptAtMs) || interruptAtMs < 0 || interruptAtMs >= duration * 1000)) fail('INVALID_INTERRUPT_TIME');
  const scenario = args['--scenario'] ?? 'conversation';
  if (!CASES.has(scenario)) fail('INVALID_SCENARIO');
  const token = environment[AUTH_ENV]?.trim();
  if (!token || /[\r\n]/.test(token)) fail('AUTH_TOKEN_REQUIRED');
  return { baseUrl: base.href.replace(/\/$/, ''), agentSessionId: args['--agent-session-id'], bindingVersion, providerId: args['--provider-id'], model: args['--model'],
    audioPath: resolve(args['--audio']), sampleRate: args['--sample-rate'] ? Number(args['--sample-rate']) : null, channels: args['--channels'] ? Number(args['--channels']) : null,
    durationMs: duration * 1000, interruptAtMs, scenario, expectWork: Boolean(args['--expect-work']), captureOutput: Boolean(args['--capture-output']),
    outputDir: resolve(args['--output'] ?? '.tmp/agent-voice-smoke'), sourceIdentity: args['--source-identity'], token };
}
export function decodeAudio(bytes, rawRate = null, rawChannels = null) {
  const buffer = Buffer.from(bytes);
  let offset = 0, size = buffer.length, rate = rawRate, channels = rawChannels;
  if (buffer.subarray(0, 4).toString() === 'RIFF' && buffer.subarray(8, 12).toString() === 'WAVE') {
    let format = null, data = null;
    for (let pos = 12; pos + 8 <= buffer.length;) {
      const id = buffer.subarray(pos, pos + 4).toString(), length = buffer.readUInt32LE(pos + 4), start = pos + 8;
      if (start + length > buffer.length) fail('TRUNCATED_WAV');
      if (id === 'fmt ') {
        if (length < 16 || buffer.readUInt16LE(start) !== 1 || buffer.readUInt16LE(start + 14) !== 16) fail('UNSUPPORTED_WAV_CODEC');
        format = { channels: buffer.readUInt16LE(start + 2), rate: buffer.readUInt32LE(start + 4) };
      }
      if (id === 'data') data = { offset: start, size: length };
      pos = start + length + (length % 2);
    }
    if (!format || !data) fail('INVALID_WAV');
    ({ rate, channels } = format); ({ offset, size } = data);
  }
  if (!Number.isInteger(rate) || rate < 8000 || rate > 192000 || !Number.isInteger(channels) || channels < 1 || channels > 8) fail('RAW_AUDIO_SPEC_REQUIRED');
  if (size % (channels * 2)) fail('PARTIAL_PCM_SAMPLE_FRAME');
  return { bytes: buffer.subarray(offset, offset + size), rate, channels };
}
export function resampleMono(audio, toRate) {
  const count = audio.bytes.length / (audio.channels * 2), mono = new Float64Array(count);
  for (let i = 0; i < count; i += 1) { let sum = 0; for (let c = 0; c < audio.channels; c += 1) sum += audio.bytes.readInt16LE((i * audio.channels + c) * 2); mono[i] = sum / audio.channels; }
  const output = Buffer.alloc(Math.floor(count * toRate / audio.rate) * 2);
  for (let i = 0; i < output.length / 2; i += 1) { const source = i * audio.rate / toRate, left = Math.floor(source), f = source - left; const sample = mono[Math.min(left, count - 1)] * (1 - f) + mono[Math.min(left + 1, count - 1)] * f; output.writeInt16LE(Math.max(-32768, Math.min(32767, Math.round(sample))), i * 2); }
  return output;
}
export function encodePacket(metadata, audio) {
  const header = Buffer.from(JSON.stringify(metadata));
  if (header.length > 4096 || audio.length > 512 * 1024) fail('MEDIA_PACKET_LIMIT');
  const packet = Buffer.alloc(8 + header.length + audio.length); packet.write('NFV1'); packet.writeUInt32LE(header.length, 4); header.copy(packet, 8); Buffer.from(audio).copy(packet, 8 + header.length); return packet;
}
export function decodePacket(value) {
  const packet = Buffer.from(value);
  if (packet.length < 8 || packet.subarray(0, 4).toString() !== 'NFV1') fail('MEDIA_PACKET_VERSION');
  const length = packet.readUInt32LE(4);
  if (length > 4096 || packet.length < length + 8 || packet.length - length - 8 > 512 * 1024) fail('MEDIA_PACKET_LIMIT');
  let metadata; try { metadata = JSON.parse(packet.subarray(8, 8 + length).toString()); } catch { fail('MEDIA_PACKET_METADATA'); }
  if (metadata.format?.encoding !== 'pcm' || metadata.format.sample_format !== 'signed16_le' || metadata.format.channels !== 1 || !Number.isSafeInteger(metadata.sequence)
    || !Number.isInteger(metadata.format.sample_rate) || metadata.format.sample_rate < 8000 || metadata.format.sample_rate > 192000) fail('UNSUPPORTED_NEGOTIATED_MEDIA');
  const bytes = packet.subarray(8 + length), actualDuration = Math.floor(bytes.length / 2 * 1000000 / metadata.format.sample_rate);
  if (bytes.length % 2 || !Number.isInteger(metadata.duration_us) || metadata.duration_us <= 0 || Math.abs(actualDuration - metadata.duration_us) > 1) fail('INVALID_PCM_DURATION');
  return { metadata, bytes };
}
export function percentile(values, fraction) { if (!values.length) return null; const ordered = [...values].sort((a, b) => a - b); return ordered[Math.ceil(ordered.length * fraction) - 1]; }
const metric = values => ({ samples: values.length, p50: percentile(values, 0.5), p95: percentile(values, 0.95) });
const target = value => value ? Object.fromEntries(['agent_session_id', 'binding_version', 'turn_operation_id', 'execution_generation'].map(key => [key, value[key]])) : null;
export function safeEvent(product) {
  if (product.kind === 'state') return { kind: 'state', state: Object.fromEntries(['voice_session_id', 'agent_session_id', 'binding_version', 'activation_epoch', 'output_generation', 'connection', 'capture', 'playback', 'work_running'].map(key => [key, product.state?.[key]])) };
  if (product.kind === 'work_receipt') return { kind: product.kind, receipt_id: product.receipt?.receipt_id, operation_key: product.receipt?.operation_key,
    upstream_trigger_id: product.upstream_trigger_id ?? product.receipt?.upstream_trigger_id ?? null, request_kind: WORK_REQUEST_KINDS.has(product.request_kind) ? product.request_kind : null,
    status: product.receipt?.status, duplicate: product.receipt?.duplicate, target: target(product.receipt?.target), pending_input_id: product.receipt?.pending_input_id ?? null };
  if (product.kind === 'approval_presented') return { kind: product.kind, presentation_id: product.presentation?.target?.presentation_id, target: target(product.presentation?.target?.work_target) };
  if (product.kind !== 'model') return { kind: product.kind };
  const event = product.event;
  switch (event?.kind) {
    case 'transcript': return { kind: event.kind, speaker: event.fragment?.speaker, fragment_id: event.fragment?.fragment_id, revision: event.fragment?.revision, commit: event.fragment?.commit, media_range: event.fragment?.media_range };
    case 'work_trigger': return { kind: event.kind, upstream_trigger_id: event.trigger?.upstream_trigger_id, trigger_kind: event.trigger?.kind,
      action: ['start', 'steer', 'observe', 'cancel', 'cancel_queued'].includes(event.trigger?.arguments?.action) ? event.trigger.arguments.action : null, context_window_ref: event.trigger?.context_window_ref ?? null };
    case 'output_started': return { kind: event.kind, segment_id: event.segment?.segment_id, response_id: event.segment?.response_id, output_generation: event.segment?.output_generation, correlation_id: event.segment?.correlation_id ?? null };
    case 'output_boundary': return { kind: event.kind, output_generation: event.output_generation };
    case 'relay_recovering': case 'relay_recovered': return { kind: event.kind, output_generation: event.output_generation };
    case 'input_mute_applied': return { kind: event.kind, muted: event.muted, request_id: event.request_id };
    case 'closed': return { kind: event.kind, reason: event.termination?.reason, finalization_confirmed: event.termination?.finalization_confirmed };
    case 'error': case 'control_rejected': return { kind: event.kind, error_kind: event.error?.kind };
    default: return { kind: event?.kind };
  }
}
function pcmToWav(bytes, format) {
  const header = Buffer.alloc(44); header.write('RIFF'); header.writeUInt32LE(bytes.length + 36, 4); header.write('WAVEfmt ', 8); header.writeUInt32LE(16, 16); header.writeUInt16LE(1, 20); header.writeUInt16LE(format.channels, 22); header.writeUInt32LE(format.sample_rate, 24); header.writeUInt32LE(format.sample_rate * format.channels * 2, 28); header.writeUInt16LE(format.channels * 2, 32); header.writeUInt16LE(16, 34); header.write('data', 36); header.writeUInt32LE(bytes.length, 40); return Buffer.concat([header, bytes]);
}
export async function runSmoke(config, dependencies = {}) {
  const fetcher = dependencies.fetch ?? globalThis.fetch, Socket = dependencies.WebSocket ?? globalThis.WebSocket;
  if (typeof fetcher !== 'function' || typeof Socket !== 'function') fail('WEBSOCKET_RUNTIME_UNAVAILABLE');
  const now = dependencies.now ?? (() => performance.now()), sleep = dependencies.sleep ?? (ms => new Promise(resolveSleep => setTimeout(resolveSleep, ms)));
  let inputBytes, fileInfo; try { fileInfo = await stat(config.audioPath); if (fileInfo.size > MAX_MEDIA_BYTES) fail('INPUT_AUDIO_LIMIT'); inputBytes = await readFile(config.audioPath); } catch (error) { if (error instanceof SmokeError) throw error; fail('INPUT_AUDIO_UNAVAILABLE'); }
  const audio = decodeAudio(inputBytes, config.sampleRate, config.channels);
  const runId = randomUUID(), trace = [], measurements = { activation_ms: [], first_audio_arrival_ms: [], interrupt_control_roundtrip_ms: [], canonical_receipt_arrival_ms: [], admission_ms: [], correction_applied_ms: [] };
  const report = { schema: 'nomifun.agent-voice-smoke.v1', run_id: runId, source_identity: config.sourceIdentity, scenario: config.scenario,
    agent_session_id: config.agentSessionId, binding_version: config.bindingVersion, provider_id: config.providerId, model: config.model,
    environment: 'headless_product_relay', evidence_source: dependencies.evidenceSource ?? 'live_product_service', implementation_result: 'running', audio_experience_result: 'pending', observations: 'Generated audio was received and discarded or explicitly captured; no audible playback or Played receipt is asserted.',
    route_identity: null, activation_epoch: null, voice_session_id: null, canonical_receipts: [], termination: null, metrics: null };
  let activation = null, socket = null, closing = false, outputBytes = 0, outputGeneration = 1, runError = null;
  let connection = 'opening', capture = 'idle', uploadAllowed = false, uploadIntent = true, flowVersion = 0;
  let recovery = null, resumeAttemptVersion = -1, resumeAbort = null, interruptAbort = null;
  const pendingInput = new Map();
  report.input_credit = { sent_frames: 0, admitted_frames: 0, released_frames: 0 };
  const invalidateResume = () => { flowVersion++; resumeAbort?.abort(); resumeAbort = null; uploadAllowed = false; };
  const sameState = state => state?.voice_session_id === activation?.voice_session_id && state?.activation_epoch === activation?.activation_epoch;
  let resolveClosed; const closedEvent = new Promise(resolveEvent => { resolveClosed = resolveEvent; });
  const redact = value => { let text = value; for (const secret of [config.token, activation?.attachment_token].filter(Boolean)) text = text.split(secret).join('[REDACTED]'); return text; };
  const captures = new Map(), triggers = new Map(), operations = new Map(), measured = new Set();
  const addTrace = entry => { if (trace.length >= 10000) fail('TRACE_EVENT_LIMIT'); trace.push({ at_ms: now() - started, ...entry }); };
  const started = now();
  const request = async (path, body, signal = undefined) => {
    let response;
    try { response = await fetcher(`${config.baseUrl}${path}`, { method: body ? 'POST' : 'GET', headers: { Authorization: `Bearer ${config.token}`, 'Content-Type': 'application/json' }, body: body ? JSON.stringify(body) : undefined, redirect: 'error', signal: signal ? AbortSignal.any([signal, AbortSignal.timeout(25000)]) : AbortSignal.timeout(25000) }); }
    catch { fail('HTTP_TRANSPORT_FAILED'); }
    if (!response.ok) fail(response.status === 401 || response.status === 403 ? 'AUTHENTICATION_REJECTED' : response.status === 409 ? 'STALE_AGENT_BINDING' : `PRODUCT_HTTP_${response.status}`);
    let envelope; try { envelope = await response.json(); } catch { fail('INVALID_PRODUCT_RESPONSE'); }
    if (!envelope?.success || !envelope.data) fail('INVALID_PRODUCT_RESPONSE');
    return envelope.data;
  };
  try {
    const catalog = await request('/api/mobile-voice/v1/catalog');
    if (!catalog.models?.some(item => item.provider_id === config.providerId && item.model === config.model && item.adapters?.some(adapter => adapter.support === 'supported' && adapter.descriptor?.transports?.includes('relay')))) fail('MODEL_CAPABILITY_UNVERIFIED');
    const availability = await request(`/api/mobile-voice/v1/availability?agent_session_id=${encodeURIComponent(config.agentSessionId)}`);
    if (!availability.enabled) fail('VOICE_NOT_CONFIGURED');
    if (availability.binding_version !== config.bindingVersion) fail('STALE_AGENT_BINDING');
    if (availability.profile?.route?.provider_id !== config.providerId || availability.profile?.route?.model !== config.model) fail('FROZEN_ROUTE_IDENTITY_MISMATCH');
    if (availability.transport !== 'relay') fail('NATIVE_DEVICE_ACCEPTANCE_REQUIRED');
    report.route_identity = { profile_id: availability.profile.profile_id, profile_revision: availability.profile.revision, route_id: availability.profile.route.route_id, route_revision: availability.profile.route.revision };
    const openingAt = now();
    activation = await request('/api/mobile-voice/v1/sessions', { agent_session_id: config.agentSessionId, binding_version: config.bindingVersion, profile_id: availability.profile.profile_id, profile_revision: availability.profile.revision, endpoint_id: `headless-smoke:${runId}`, transport: 'relay', takeover: false });
    measurements.activation_ms.push(now() - openingAt); report.voice_session_id = activation.voice_session_id; report.activation_epoch = activation.activation_epoch;
    const spec = activation.negotiation?.input_spec;
    if (!spec || spec.format.encoding !== 'pcm' || spec.format.sample_format !== 'signed16_le' || spec.format.channels !== 1) fail('UNSUPPORTED_NEGOTIATED_MEDIA');
    const pcm = resampleMono(audio, spec.format.sample_rate);
    const cadenceUs = Math.max(spec.min_frame_duration_us, Math.min(spec.max_frame_duration_us, 20000));
    const frameBytes = Math.floor(spec.format.sample_rate * cadenceUs / 1000000) * 2;
    if (!frameBytes || frameBytes > spec.max_frame_bytes) fail('INVALID_NEGOTIATED_CADENCE');
    const url = new URL(`${config.baseUrl}/api/mobile-voice/v1/sessions/${encodeURIComponent(activation.voice_session_id)}/media`); url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
    let openedResolve, openedReject; const opened = new Promise((r, j) => { openedResolve = r; openedReject = j; });
    socket = new Socket(url.href, ['nomifun.voice.v1', `nomifun.voice.ticket.${activation.attachment_token}`]); socket.binaryType = 'arraybuffer';
    const mediaStarted = now(); let firstAudio = true;
    socket.onopen = () => openedResolve(); socket.onerror = () => { runError = new SmokeError('MEDIA_TRANSPORT_FAILED'); openedReject(runError); };
    socket.onclose = () => { if (!closing) runError = new SmokeError('MEDIA_CLOSED_BEFORE_REQUESTED_CLOSE'); };
    socket.onmessage = message => {
      try {
        if (typeof message.data === 'string') {
          const product = JSON.parse(message.data), safe = safeEvent(product); addTrace(safe);
          if (['input_admitted', 'input_released'].includes(product.kind) && product.activation_epoch === activation.activation_epoch && Number.isSafeInteger(product.sequence)) {
            const pending = pendingInput.get(product.sequence);
            if (pending && product.duration_us === pending.duration_us) {
              pendingInput.delete(product.sequence); report.input_credit[product.kind === 'input_admitted' ? 'admitted_frames' : 'released_frames']++;
            }
          }
          if (safe.kind === 'state' && sameState(product.state)) {
            if (recovery && product.state.output_generation > recovery.generation) { invalidateResume(); recovery = null; }
            connection = product.state.connection; capture = product.state.capture; outputGeneration = product.state.output_generation;
            if (['closing', 'closed', 'failed'].includes(connection)) { invalidateResume(); if (!closing) runError = new SmokeError('VOICE_CLOSED_BEFORE_REQUESTED_CLOSE'); }
            else if (connection === 'recovering') { uploadAllowed = false; }
            else if (connection === 'ready' && capture === 'capturing' && !recovery && uploadIntent) uploadAllowed = true;
            else if (capture !== 'capturing') uploadAllowed = false;
          }
          if (product.kind === 'endpoint_control' && product.activation_epoch === activation.activation_epoch) {
            if (product.control?.kind === 'close') { invalidateResume(); if (!closing) runError = new SmokeError('VOICE_CLOSED_BEFORE_REQUESTED_CLOSE'); }
            if (product.control?.kind === 'mute_input' && product.control.muted) uploadAllowed = false;
          }
          if (safe.kind === 'relay_recovering' && Number.isSafeInteger(safe.output_generation) && safe.output_generation >= outputGeneration) {
            invalidateResume(); outputGeneration = safe.output_generation; recovery = { generation: outputGeneration, version: flowVersion, recovered: false };
          }
          if (safe.kind === 'relay_recovered' && recovery && safe.output_generation === outputGeneration && safe.output_generation === recovery.generation && recovery.version === flowVersion) recovery.recovered = true;
          if (safe.kind === 'input_mute_applied' && safe.muted) { uploadIntent = false; invalidateResume(); }
          if (safe.kind === 'work_trigger' && safe.upstream_trigger_id && !triggers.has(safe.upstream_trigger_id)) triggers.set(safe.upstream_trigger_id, { at: now() });
          if (safe.kind === 'work_receipt') {
            report.canonical_receipts.push(safe); measurements.canonical_receipt_arrival_ms.push(now() - mediaStarted);
            const prior = operations.get(safe.operation_key), trigger = safe.upstream_trigger_id ? triggers.get(safe.upstream_trigger_id) : null;
            if (safe.operation_key && (prior || trigger)) operations.set(safe.operation_key, { at: prior?.at ?? trigger.at, request_kind: safe.request_kind ?? prior?.request_kind ?? null });
            const operation = operations.get(safe.operation_key);
            if (operation?.request_kind === 'start' && safe.status === 'accepted' && !measured.has(`admission:${safe.operation_key}`)) { measurements.admission_ms.push(now() - operation.at); measured.add(`admission:${safe.operation_key}`); }
            if (operation?.request_kind === 'steer' && safe.status === 'applied' && !measured.has(`correction:${safe.operation_key}`)) { measurements.correction_applied_ms.push(now() - operation.at); measured.add(`correction:${safe.operation_key}`); }
          }
          if (safe.kind === 'closed') { report.termination = safe; resolveClosed(); }
          if (safe.kind === 'error') runError = new SmokeError('VOICE_PROVIDER_OR_APPLICATION_FAILED');
        } else {
          const { metadata, bytes } = decodePacket(message.data); outputBytes += bytes.length;
          if (outputBytes > MAX_MEDIA_BYTES) fail('OUTPUT_AUDIO_LIMIT');
          if (metadata.activation_epoch !== activation.activation_epoch || metadata.output_generation < outputGeneration) { addTrace({ kind: 'discarded_stale_audio', sequence: metadata.sequence }); return; }
          if (firstAudio) { measurements.first_audio_arrival_ms.push(now() - mediaStarted); firstAudio = false; }
          addTrace({ kind: 'generated_audio_received', segment_id: metadata.segment_id, output_generation: metadata.output_generation, sequence: metadata.sequence, duration_us: metadata.duration_us, format: metadata.format, playback: 'not_observed' });
          if (config.captureOutput) { const key = String(metadata.output_generation); if (!captures.has(key)) captures.set(key, { format: metadata.format, chunks: [] }); captures.get(key).chunks.push(Buffer.from(bytes)); }
        }
      } catch (error) { runError = error instanceof SmokeError ? error : new SmokeError('INVALID_MEDIA_EVENT'); }
    };
    let openTimer; try { await Promise.race([opened, new Promise((_, reject) => { openTimer = setTimeout(() => reject(new SmokeError('MEDIA_OPEN_DEADLINE')), 10000); })]); } finally { clearTimeout(openTimer); }
    let sequence = 0, offset = 0, interrupted = false, heartbeatSequence = 0, nextHeartbeat = 0;
    while (now() - mediaStarted < config.durationMs && !runError) {
      const elapsed = now() - mediaStarted;
      if (elapsed >= nextHeartbeat) { socket.send(JSON.stringify({kind: 'foreground', activation_epoch: activation.activation_epoch, sequence: ++heartbeatSequence})); nextHeartbeat = elapsed + 1000; }
      if (recovery?.recovered && recovery.version === flowVersion && connection === 'ready' && capture === 'paused' && uploadIntent && resumeAttemptVersion !== flowVersion) {
        const expected = recovery; resumeAttemptVersion = flowVersion; const abort = new AbortController(); resumeAbort = abort;
        void request(`/api/mobile-voice/v1/sessions/${encodeURIComponent(activation.voice_session_id)}/control`, { activation_epoch: activation.activation_epoch, control: { kind: 'mute_input', muted: false } }, abort.signal).then(state => {
          if (closing || abort.signal.aborted || recovery !== expected || flowVersion !== expected.version || !uploadIntent) return;
          if (!sameState(state) || state.output_generation !== expected.generation || state.connection !== 'ready' || state.capture !== 'capturing') { runError = new SmokeError('INPUT_RESUME_ACK_MISMATCH'); return; }
          connection = state.connection; capture = state.capture; outputGeneration = state.output_generation; recovery = null; uploadAllowed = true;
          addTrace({ kind: 'input_resumed_after_provider_ack', output_generation: outputGeneration });
        }).catch(error => { if (!closing && !abort.signal.aborted && recovery === expected && flowVersion === expected.version) runError = error; }).finally(() => { if (resumeAbort === abort) resumeAbort = null; });
      }
      if (!interrupted && config.interruptAtMs !== null && elapsed >= config.interruptAtMs) {
        interrupted = true; const at = now(); const abort = new AbortController(); interruptAbort = abort;
        void request(`/api/mobile-voice/v1/sessions/${encodeURIComponent(activation.voice_session_id)}/control`, { activation_epoch: activation.activation_epoch, control: { kind: 'interrupt_output', output_generation: outputGeneration + 1, played: null } }, abort.signal).then(state => {
          if (closing || abort.signal.aborted) return;
          if (!sameState(state)) { runError = new SmokeError('INTERRUPT_ACK_MISMATCH'); return; }
          outputGeneration = Math.max(outputGeneration, state.output_generation); measurements.interrupt_control_roundtrip_ms.push(now() - at); addTrace({ kind: 'interrupt_control_acknowledged', output_generation: state.output_generation, local_audible_stop_ms: null });
        }).catch(error => { if (!closing && !abort.signal.aborted) runError = error; }).finally(() => { if (interruptAbort === abort) interruptAbort = null; });
      }
      if (!uploadAllowed || connection !== 'ready' || capture !== 'capturing') { await sleep(cadenceUs / 1000); continue; }
      if (pendingInput.size >= 4) fail('REMOTE_INPUT_CREDIT_EXHAUSTED');
      const frame = Buffer.alloc(frameBytes); if (offset < pcm.length) pcm.copy(frame, 0, offset, Math.min(offset + frameBytes, pcm.length)); offset += frameBytes;
      if (socket.readyState !== 1) fail('MEDIA_NOT_READY'); if (socket.bufferedAmount > spec.max_frame_bytes * 4) fail('LOCAL_UPLINK_BACKLOG');
      const durationUs = Math.floor(frameBytes / 2 * 1000000 / spec.format.sample_rate); pendingInput.set(++sequence, { duration_us: durationUs }); report.input_credit.sent_frames++;
      socket.send(encodePacket({ activation_epoch: activation.activation_epoch, output_generation: outputGeneration, sequence, timestamp: Math.round(elapsed * 1000), duration_us: durationUs, format: spec.format }, frame));
      await sleep(cadenceUs / 1000);
    }
    if (runError) throw runError;
    if (!outputBytes) fail('NO_GENERATED_AUDIO_OBSERVED');
    if (config.expectWork && !report.canonical_receipts.some(receipt => ['accepted', 'applied', 'pending_boundary', 'queued', 'terminal'].includes(receipt.status))) fail('NO_CANONICAL_WORK_RECEIPT');
    report.implementation_result = dependencies.evidenceSource === 'local_fixture' ? 'local_runner_test_passed' : 'wire_and_receipt_observation_passed';
  } catch (error) { report.implementation_result = 'failed'; report.failure_code = error instanceof SmokeError ? error.code : 'UNEXPECTED_FAILURE'; }
  finally {
    closing = true; invalidateResume(); interruptAbort?.abort(); interruptAbort = null;
    if (activation) {
      try { await request(`/api/mobile-voice/v1/sessions/${encodeURIComponent(activation.voice_session_id)}/control`, { activation_epoch: activation.activation_epoch, control: { kind: 'close', reason: 'user_ended' } }); }
      catch { report.failure_code ??= 'VOICE_CLOSE_REQUEST_FAILED'; report.implementation_result = 'failed'; }
      if (!report.termination) { let closeTimer; try { await Promise.race([closedEvent, new Promise(resolveTimeout => { closeTimer = setTimeout(resolveTimeout, 3000); })]); } finally { clearTimeout(closeTimer); } }
    }
    socket?.close();
    if (activation && !report.termination?.finalization_confirmed) { report.implementation_result = 'failed'; report.failure_code ??= 'FINALIZATION_UNCONFIRMED'; }
    report.metrics = Object.fromEntries(Object.entries(measurements).map(([name, values]) => [name, metric(values)]));
    report.metrics.correction_applied_ms.measurement = 'work_trigger_received_to_canonical_applied';
    report.metrics.correction_applied_ms.classification = 'application_executed_request_kind';
    report.metrics.local_audible_stop_ms = { samples: 0, p50: null, p95: null, status: 'requires_real_endpoint' };
    report.metrics.first_audible_response_ms = { samples: 0, p50: null, p95: null, status: 'requires_real_endpoint' };
    report.metrics.playback_backlog_us = null; report.metrics.playback_underflows = null;
    report.generated_audio_bytes = outputBytes;
    report.work_continuity = 'Ending voice did not issue any work cancel request.';
    await mkdir(config.outputDir, { recursive: true });
    await writeFile(resolve(config.outputDir, `${runId}.trace.jsonl`), redact(trace.map(entry => JSON.stringify(entry)).join('\n') + '\n'));
    await writeFile(resolve(config.outputDir, `${runId}.report.json`), redact(JSON.stringify(report, null, 2) + '\n'));
    if (config.captureOutput) for (const [generation, capture] of captures) await writeFile(resolve(config.outputDir, `${runId}.generated-g${generation}.wav`), pcmToWav(Buffer.concat(capture.chunks), capture.format));
  }
  return JSON.parse(redact(JSON.stringify(report)));
}
const HELP = `Bounded, opt-in relay wire smoke. It neither opens a GUI nor asserts real audio experience.\n\nSet ${AUTH_ENV} to an existing authorized NomiFun Web token. Supply an isolated voice-enabled AgentSession, its frozen provider/model and a PCM16 WAV (or raw PCM with --sample-rate/--channels).\n\nbun scripts/validation/run-agent-voice-smoke.mjs --run-live --base-url http://127.0.0.1:PORT --agent-session-id SESSION --binding-version VERSION --provider-id PROVIDER --model MODEL --source-identity COMMIT --audio fixture.wav [--duration 30] [--expect-work] [--interrupt-at-ms 5000] [--capture-output] [--scenario correction] [--output DIR]\n\nMedia and transcripts can drive work under that Agent's canonical grants. Voice close keeps admitted work running. Trace files omit transcripts, tool arguments, summaries, tokens and raw provider diagnostics. Generated WAV capture is opt-in and is not a played receipt. Native/device/headset/AEC/GUI acceptance remains pending.\n`;
if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  try { const config = parseArgs(process.argv.slice(2)); if (config.help) process.stdout.write(HELP); else { const report = await runSmoke(config); process.stdout.write(JSON.stringify(report) + '\n'); if (report.implementation_result === 'failed') process.exitCode = 1; } }
  catch (error) { process.stderr.write(JSON.stringify({ schema: 'nomifun.agent-voice-smoke.v1', implementation_result: 'failed', audio_experience_result: 'pending', failure_code: error instanceof SmokeError ? error.code : 'UNEXPECTED_FAILURE' }) + '\n'); process.exitCode = 1; }
}
