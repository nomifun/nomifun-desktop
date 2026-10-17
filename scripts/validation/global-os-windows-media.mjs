#!/usr/bin/env node
// Metadata-only media acquisition in the actual disposable native WebView2.
// No recording, sample reads, media uploads, permission overrides or resets.
import assert from 'node:assert/strict';
import { createHash, randomUUID } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { readFile, realpath, writeFile } from 'node:fs/promises';
import { basename, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { assertWindowsPrivatePath, windowsProcessGone } from './global-os-windows-cases.mjs';

const delay = ms => new Promise(done => setTimeout(done, ms));
async function evaluate(target, expression) {
  const socket = new WebSocket(target.webSocketDebuggerUrl);
  try {
    await new Promise((done, fail) => {
      const timer = setTimeout(() => fail(new Error('Native fixture CDP connection timed out')), 5000);
      socket.addEventListener('open', () => { clearTimeout(timer); done(); }, { once: true });
      socket.addEventListener('error', () => { clearTimeout(timer); fail(new Error('Native fixture CDP connection failed')); }, { once: true });
      socket.addEventListener('close', () => { clearTimeout(timer); fail(new Error('Native fixture CDP connection closed')); }, { once: true });
    });
    return await new Promise((done, fail) => {
      const timeout = setTimeout(() => fail(new Error('Native fixture media operation timed out')), 12000);
      socket.addEventListener('close', () => { clearTimeout(timeout); fail(new Error('Native fixture media result was not received')); }, { once: true });
      socket.addEventListener('message', event => {
        let value; try { value = JSON.parse(event.data); } catch { return; }
        if (value.id !== 1) return;
        clearTimeout(timeout);
        if (value.error || value.result?.exceptionDetails) { fail(new Error('Native fixture media operation failed; private response omitted')); return; }
        done(value.result.result.value);
      });
      socket.send(JSON.stringify({ id: 1, method: 'Runtime.evaluate', params: { expression, awaitPromise: true, returnByValue: true } }));
    });
  } finally { socket.close(); }
}
async function digest(path) { const hash = createHash('sha256'); for await (const bytes of createReadStream(path)) hash.update(bytes); return hash.digest('hex'); }

// Inject this exact function into the verified native renderer. Protocol tests
// exercise the same function with fake tracks/contexts, never real devices.
export function startWindowsNativeMediaProbe(key, kind) {
  if (window[key]) throw new Error('Probe collision');
  const state = { kind, status: 'pending', created_at: performance.now(), cancelled: false, stream_acquired: false, context_created: false, cleanup_proven: false, cleanup_errors: [] };
  window[key] = state;
  let resolveCancellation;
  const cancellation = new Promise(done => { resolveCancellation = done; });
  const record = (stage, error) => state.cleanup_errors.push({ stage, name: error?.name ?? 'Error', message: error?.message ?? String(error) });
  const stopTracks = () => {
    for (const track of state.raw_tracks ?? []) try { track.stop(); } catch (error) { record('track.stop', error); }
  };
  const closeContext = () => {
    if (!state.context_created) return Promise.resolve();
    state.context_close ??= Promise.resolve().then(() => state.context.close());
    // Cancellation can start close while capture still awaits resume. Attach
    // an observer immediately; the same promise is joined in final cleanup.
    state.context_close.catch(() => {});
    return state.context_close;
  };
  state.snapshot = () => ({ kind: state.kind, status: state.status, capture_status: state.capture_status, cancelled: state.cancelled,
    stream_acquired: state.stream_acquired, context_created: state.context_created, tracks: state.tracks, graph: state.graph,
    error: state.error, cleanup_errors: state.cleanup_errors.map(value => ({ ...value })), elapsed_ms: state.elapsed_ms,
    all_tracks_ended: state.all_tracks_ended, context_closed: state.context_closed, cleanup_proven: state.cleanup_proven });
  state.cancel = () => { state.cancelled = true; resolveCancellation(); stopTracks(); closeContext().catch(() => {}); };
  state.pending = (async () => {
    let outcome = 'failed';
    try {
      const stream = await navigator.mediaDevices.getUserMedia(kind === 'audio' ? { audio: true, video: false } : { audio: false, video: true });
      state.stream_acquired = true; state.stream = stream; state.raw_tracks = stream.getTracks();
      state.tracks = state.raw_tracks.map(track => { const settings = track.getSettings(); return {
        kind: track.kind, ready_state: track.readyState, enabled: track.enabled, muted: track.muted,
        sample_rate: settings.sampleRate, channel_count: settings.channelCount, width: settings.width, height: settings.height, frame_rate: settings.frameRate,
      }; });
      if (state.cancelled) { outcome = 'cancelled'; return; }
      if (!state.tracks.length || state.tracks.some(track => track.kind !== kind || track.ready_state !== 'live')) {
        throw new Error('Native capture did not yield live requested tracks');
      }
      if (kind === 'audio') {
        state.context = new AudioContext(); state.context_created = true;
        state.source = state.context.createMediaStreamSource(stream); state.gain = state.context.createGain(); state.gain.gain.value = 0;
        state.source.connect(state.gain); state.gain.connect(state.context.destination);
        let resumeTimer;
        try { await Promise.race([state.context.resume(), cancellation, new Promise((_, fail) => { resumeTimer = setTimeout(() => fail(new Error('AudioContext resume needs a real user action')), 5000); })]); }
        finally { clearTimeout(resumeTimer); }
        if (state.cancelled) { outcome = 'cancelled'; return; }
        const before = state.context.currentTime;
        await Promise.race([new Promise(done => setTimeout(done, 250)), cancellation]);
        if (state.cancelled) { outcome = 'cancelled'; return; }
        state.graph = { connected: true, silent_output: true, state: state.context.state, time_advanced: state.context.currentTime > before };
        if (state.context.state !== 'running' || !state.graph.time_advanced) throw new Error('Actual audio graph did not run');
      }
      if (!state.raw_tracks.every(track => track.readyState === 'live')) throw new Error('Native capture ended before its final observation');
      outcome = 'acquired';
    } catch (error) {
      if (state.cancelled) outcome = 'cancelled';
      else state.error = { name: error.name, message: error.message };
    } finally {
      stopTracks();
      for (const [name, node] of [['source.disconnect', state.source], ['gain.disconnect', state.gain]]) {
        if (node) try { node.disconnect(); } catch (error) { record(name, error); }
      }
      if (state.context_created) {
        try { await closeContext(); } catch (error) { record('context.close', error); }
        state.context_closed = state.context.state === 'closed';
      }
      state.all_tracks_ended = !state.stream_acquired || (Array.isArray(state.raw_tracks) && state.raw_tracks.every(track => track.readyState === 'ended'));
      state.cleanup_proven = state.all_tracks_ended && (!state.context_created || state.context_closed === true);
      state.capture_status = state.cancelled ? 'cancelled' : outcome;
      state.elapsed_ms = Math.round(performance.now() - state.created_at);
      // Publish the only terminal status after every cleanup attempt and the
      // actual close promise. Pending getUserMedia/close remain pending.
      state.status = !state.cleanup_proven ? 'cleanup_unproven' : state.cleanup_errors.length ? 'failed' : state.capture_status;
    }
  })().catch(error => {
    record('unexpected_probe_failure', error); state.cleanup_proven = false; state.status = 'cleanup_unproven';
  });
  return state.snapshot();
}

export async function cancelWindowsNativeMediaProbe(key, joinMs = 5000) {
  const state = window[key];
  if (!state) return { status: 'state_lost', cleanup_proven: false, joined: false };
  state.cancel();
  let timer;
  try { await Promise.race([state.pending, new Promise(done => { timer = setTimeout(done, joinMs); })]); }
  finally { clearTimeout(timer); }
  return { ...state.snapshot(), joined: state.status !== 'pending', late_acquisition_will_stop: state.cancelled };
}

export async function verifyWindowsNativeMedia({ fixtureReport, signal }) {
  assert.equal(process.platform, 'win32');
  const fixture = JSON.parse(await readFile(fixtureReport, 'utf8'));
  assert.equal(fixture.schema, 'nomifun.windows-global-os-fixture.v1'); assert.equal(fixture.status, 'ready');
  assert.match(basename(fixture.parent), /^nomifun-global-os-owner-[0-9a-f-]{36}$/);
  assert.equal(fixture.native_data_root, join(fixture.parent, 'native-data'));
  assert.equal(await realpath(fixture.parent), fixture.parent);
  await assertWindowsPrivatePath(fixture.parent); await assertWindowsPrivatePath(fixture.native_data_root);
  assert.equal(await readFile(join(fixture.parent, '.nomifun-global-os-fixture'), 'utf8'), 'nomifun-global-os-native-acceptance-fixture.v1\n');
  assert(!windowsProcessGone(fixture.app_pid));
  assert.equal(JSON.parse(await readFile(join(fixture.native_data_root, 'port.json'), 'utf8')).pid, fixture.app_pid);
  assert.equal(await digest(fixture.exe), fixture.exe_sha256);
  const targets = await (await fetch(`http://127.0.0.1:${fixture.cdp_port}/json/list`, { signal: AbortSignal.timeout(5000) })).json();
  const target = targets.find(value => { try { return value.type === 'page' && new URL(value.url).origin === fixture.renderer.origin; } catch { return false; } });
  assert(target, 'The exact disposable native renderer must be present');
  const debuggerUrl = new URL(target.webSocketDebuggerUrl);
  assert.equal(debuggerUrl.protocol, 'ws:'); assert(['127.0.0.1', 'localhost', '[::1]'].includes(debuggerUrl.hostname));
  assert.equal(Number(debuggerUrl.port), fixture.cdp_port); assert(!debuggerUrl.username && !debuggerUrl.password);
  const identity = await evaluate(target, "(async()=>({version:await window.__TAURI_INTERNALS__.invoke('plugin:app|version'),origin:location.origin,secure_context:isSecureContext,width:innerWidth,height:innerHeight}))()");
  assert.equal(identity.origin, fixture.renderer.origin); assert.equal(identity.version, fixture.renderer.version);
  assert.equal(identity.secure_context, true);
  assert(identity.width >= 880 && identity.height >= 600);
  const key = `__nomifun_native_media_${randomUUID().replaceAll('-', '')}`;
  const report = { schema: 'nomifun.windows-native-media.v1', status: 'running', fixture_report: resolve(fixtureReport), app_pid: fixture.app_pid, exe_sha256: fixture.exe_sha256, native_renderer: identity, attempts: [], scope: 'Actual WebView2 getUserMedia track/graph/lifecycle metadata; no media content or model/ASR proof', created_at: new Date().toISOString() };
  const reportPath = join(fixture.report_root, `windows-native-media-${randomUUID()}.json`);
  const interruption = new AbortController();
  const interrupted = () => interruption.abort(new Error('Native media verification interrupted'));
  process.on('SIGINT', interrupted); process.on('SIGTERM', interrupted); signal?.addEventListener('abort', interrupted, { once: true });
  if (signal?.aborted) interrupted();
  let activeProbe = false;
  const snapshot = () => evaluate(target, `(()=>{const s=window[${JSON.stringify(key)}];return s?s.snapshot():{status:'state_lost',cleanup_proven:false}})()`);
  const cancelAndJoin = () => evaluate(target, `(${cancelWindowsNativeMediaProbe.toString()})(${JSON.stringify(key)},5000)`);
  try {
    report.inventory = await evaluate(target, "(async()=>{const all=await navigator.mediaDevices.enumerateDevices();return {audio_inputs:all.filter(x=>x.kind==='audioinput').length,video_inputs:all.filter(x=>x.kind==='videoinput').length,audio_outputs:all.filter(x=>x.kind==='audiooutput').length};})()");
    for (const kind of ['audio', 'video']) {
      if (interruption.signal.aborted) { report.status = 'cancelled'; break; }
      activeProbe = true; // Dispatch loss cannot prove that getUserMedia never started.
      await evaluate(target, `(${startWindowsNativeMediaProbe.toString()})(${JSON.stringify(key)},${JSON.stringify(kind)})`);
      let result;
      for (let attempt = 0; attempt < 180; attempt++) {
        if (interruption.signal.aborted) break;
        await delay(500);
        result = await snapshot();
        if (result.status !== 'pending') break;
      }
      if (!result || result.status === 'pending' || interruption.signal.aborted) {
        const before = result;
        result = await cancelAndJoin();
        report.attempts.push({ ...result, timed_out_observation: before, cancellation_requested: true });
        report.status = interruption.signal.aborted && result.cleanup_proven ? 'cancelled' : 'pending_native_result';
        break;
      }
      report.attempts.push(result);
      assert(result.cleanup_proven, 'Actual track/context cleanup must be proven');
      await evaluate(target, `delete window[${JSON.stringify(key)}]`);
      activeProbe = false;
    }
    if (report.status === 'running') report.status = report.attempts.every(value => value.status === 'acquired') ? 'passed' : 'capture_not_established';
  } catch (error) { report.status = 'failed'; report.failure = { name: error.name, message: error.message }; throw error; }
  finally {
    if (activeProbe) {
      try { report.final_cleanup_observation = await cancelAndJoin(); }
      catch (error) { report.final_cleanup_observation = { cleanup_proven: false, status: 'native_result_unknown', error: { name: error.name, message: error.message } }; }
    }
    process.removeListener('SIGINT', interrupted); process.removeListener('SIGTERM', interrupted); signal?.removeEventListener('abort', interrupted);
    await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`, { flag: 'wx' });
    console.log(JSON.stringify({ status: report.status, report: reportPath, attempts: report.attempts.map(({ kind, status, cleanup_proven }) => ({ kind, status, cleanup_proven })) }));
  }
  return report;
}
if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const at = process.argv.indexOf('--fixture-report');
  assert(at >= 0 && process.argv[at + 1], '--fixture-report must name the existing disposable native App');
  await verifyWindowsNativeMedia({ fixtureReport: process.argv[at + 1] });
}
