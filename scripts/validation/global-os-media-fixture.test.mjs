import { expect, test } from 'bun:test';
import { spawn } from 'node:child_process';
import { mkdtemp, readFile, readdir, realpath, rm, stat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createContext, Script } from 'node:vm';
import { buildMediaFixturePage, sanitizeMediaResult } from './global-os-media-fixture.mjs';

// These fixtures describe a denied request. They never claim that a device
// captured data, and the HTTP tests never submit a synthetic device PASS.
const captureModes = ['audio_video', 'audio', 'video'];
const buttonIds = { audio_video: 'start-both', audio: 'start-audio', video: 'start-video' };
const errorConstraints = ['width', 'height', 'aspectRatio', 'frameRate', 'facingMode', 'resizeMode', 'sampleRate', 'sampleSize',
  'echoCancellation', 'autoGainControl', 'noiseSuppression', 'latency', 'channelCount', 'volume'];

function deniedMetadata(attempt = 1, captureMode = 'audio_video') {
  return { schema: 'nomifun.local-media-attempt.v2', capture_mode: captureMode, error_constraint: '',
    attempt, status: 'failed', reason: 'capture_failed', error_name: 'NotAllowedError',
    secure_context: true, tracks_live_before_stop: false, video_loaded_data: false, video_frame_callback_available: false,
    video_frame_callback_received: false, audio_graph_connected: false, audio_context_running: false, audio_context_time_advanced: false,
    all_tracks_ended: true, audio_context_closed: true, audio_tracks: 0, video_tracks: 0, total_tracks: 0,
    video_width: 0, video_height: 0, audio_sample_rate: 0, analyser_fft_size: 0, elapsed_ms: 0 };
}

test('rejects invented PASS, raw capture fields, and unbounded or ill-typed metadata', () => {
  for (const captureMode of captureModes) {
    expect(sanitizeMediaResult(deniedMetadata(1, captureMode)).capture_mode).toBe(captureMode);
    expect(() => sanitizeMediaResult({ ...deniedMetadata(1, captureMode), status: 'passed', reason: 'complete', error_name: undefined })).toThrow('A media PASS requires');
  }
  const { capture_mode, error_constraint, ...legacy } = deniedMetadata();
  expect(sanitizeMediaResult({ ...legacy, schema: 'nomifun.local-media-attempt.v1' }).status).toBe('failed');
  for (const field of ['deviceId', 'label', 'samples', 'image', 'audio', 'video', 'raw', 'error_message', 'constraint_details']) {
    expect(() => sanitizeMediaResult({ ...deniedMetadata(), [field]: 'forbidden' })).toThrow('Unexpected fields');
  }
  for (const invalid of [{ attempt: 0 }, { attempt: 33 }, { video_width: -1 }, { elapsed_ms: 10_000_001 },
    { audio_tracks: 0.5 }, { all_tracks_ended: 'true' }, { schema: 'unknown' }, { reason: 'unknown' },
    { capture_mode: undefined }, { capture_mode: 'screen' }, { capture_mode: null }, { error_constraint: undefined },
    { error_constraint: null }, { error_constraint: 'deviceId' }, { error_constraint: 'arbitrary-private-detail' }]) {
    expect(() => sanitizeMediaResult({ ...deniedMetadata(), ...invalid })).toThrow();
  }
});

test('retains only the bounded standard OverconstrainedError constraint vocabulary', () => {
  for (const constraint of ['', 'unknown', ...errorConstraints]) {
    const clean = sanitizeMediaResult({ ...deniedMetadata(), error_name: 'OverconstrainedError', error_constraint: constraint });
    expect(clean.error_name).toBe('OverconstrainedError');
    expect(clean.error_constraint).toBe(constraint);
  }
});

test('compiles the actual page script and keeps media content out of all output paths', () => {
  const nonce = 'a'.repeat(48), token = 'b'.repeat(48);
  const html = buildMediaFixturePage({ pageToken: token, cspNonce: nonce });
  const script = html.match(/<script nonce="([a-f0-9]+)">([\s\S]*?)<\/script>/);
  expect(script?.[1]).toBe(nonce);
  expect(() => new Script(script[2])).not.toThrow();
  expect(html.match(/nonce="a{48}"/g)?.length).toBe(2);
  for (const id of Object.values(buttonIds)) expect(html).toContain(`id="${id}"`);
  expect(script[2]).toMatch(/navigator\.mediaDevices\.getUserMedia\(\{\s*audio:\s*wantsAudio,\s*video:\s*wantsVideo\s*\}\)/);
  expect(script[2]).toContain(`fetch('/${token}/result'`);
  expect(script[2]).toContain('body:JSON.stringify(result)');
  expect(script[2]).not.toMatch(/MediaRecorder|\.getSettings\(|\.getCapabilities\(|enumerateDevices|\.deviceId|\.label|\.destination|\.get(?:Byte|Float)(?:TimeDomain|Frequency)Data\(|createElement\(['"]canvas|toDataURL|toBlob|appendChild|append\(/);
  expect(() => buildMediaFixturePage({ pageToken: '../invalid', cspNonce: nonce })).toThrow();
});

function pageScript() {
  const html = buildMediaFixturePage({ pageToken: 'b'.repeat(48), cspNonce: 'a'.repeat(48) });
  return html.match(/<script nonce="[a-f0-9]+">([\s\S]*?)<\/script>/)[1];
}

function pageElements() {
  const handlers = {};
  const elements = Object.fromEntries(Object.values(buttonIds).map(id => [id,
    { disabled: false, addEventListener: (_event, handler) => { handlers[id] = handler; } }]));
  return { handlers, elements: { ...elements, status: {}, result: {} } };
}

for (const captureMode of captureModes) test(`${captureMode} records only a sanitized failed OverconstrainedError and stops its audio graph`, async () => {
  for (const [constraint, expectedConstraint] of [['width', 'width'], ['sampleRate', 'sampleRate'], ['private-device-identifier', 'unknown']]) {
    const { handlers, elements } = pageElements();
    let result, requested, constructed = 0, closed = 0;
    class StubAudioContext {
      state = 'running';
      constructor() { constructed++; }
      resume() { return Promise.resolve(); }
      close() { this.state = 'closed'; closed++; return Promise.resolve(); }
    }
    const captureError = Object.assign(new Error('PRIVATE_DEVICE_ERROR_MUST_NEVER_BE_RETAINED'), { name: 'OverconstrainedError', constraint });
    const context = createContext({
      document: { hidden: false, getElementById: id => elements[id], addEventListener() {},
        createElement: () => { throw new Error('A rejected capture must not create a decoder'); } },
      window: { isSecureContext: true, ...(captureMode === 'video' ? {} : { AudioContext: StubAudioContext }), addEventListener() {} },
      ...(captureMode === 'video' ? {} : { AudioContext: StubAudioContext }),
      navigator: { mediaDevices: { getUserMedia: async constraints => { requested = constraints; throw captureError; } } },
      performance, DOMException, setTimeout: (handler, delay) => setTimeout(handler, Math.min(delay, 5)), clearTimeout,
      fetch: async (_url, request) => { result = JSON.parse(request.body); return { ok: true }; },
    });
    new Script(pageScript()).runInContext(context);
    await handlers[buttonIds[captureMode]]();
    expect({ ...requested }).toEqual({ audio: captureMode !== 'video', video: captureMode !== 'audio' });
    expect(constructed).toBe(captureMode === 'video' ? 0 : 1);
    expect(closed).toBe(constructed);
    expect(result.schema).toBe('nomifun.local-media-attempt.v2');
    expect(result.capture_mode).toBe(captureMode);
    expect(result.status).toBe('failed');
    expect(result.error_name).toBe('OverconstrainedError');
    expect(result.error_constraint).toBe(expectedConstraint);
    expect(result.all_tracks_ended).toBe(true);
    expect(result.audio_context_closed).toBe(true);
    expect(JSON.stringify(result)).not.toContain('PRIVATE_DEVICE_ERROR');
    expect(JSON.stringify(result)).not.toContain('private-device-identifier');
    expect(sanitizeMediaResult(result).status).toBe('failed');
    for (const id of Object.values(buttonIds)) expect(elements[id].disabled).toBe(false);
  }
});

for (const captureMode of ['audio_video', 'video']) test(`${captureMode} pending video play still stops every stub track and closes its audio graph`, async () => {
  const { handlers, elements } = pageElements();
  let result, stopped = 0, constructed = 0, closed = 0;
  const tracks = (captureMode === 'video' ? ['video'] : ['audio', 'video']).map(kind =>
    ({ kind, readyState: 'live', stop() { stopped++; this.readyState = 'ended'; } }));
  const stream = { getTracks: () => tracks, getAudioTracks: () => tracks.filter(track => track.kind === 'audio'),
    getVideoTracks: () => tracks.filter(track => track.kind === 'video') };
  class StubAudioContext {
    state = 'running'; sampleRate = 48000; currentTime = 0;
    constructor() { constructed++; }
    resume() { return Promise.resolve(); }
    createMediaStreamSource() { return { connect() {}, disconnect() {} }; }
    createAnalyser() { return { fftSize: 0, disconnect() {} }; }
    close() { this.state = 'closed'; closed++; return Promise.resolve(); }
  }
  const context = createContext({
    document: { hidden: false, getElementById: id => elements[id], addEventListener() {},
      createElement: () => ({ readyState: 2, videoWidth: 640, videoHeight: 480,
        play: () => new Promise(() => {}), pause() {}, srcObject: undefined }) },
    window: { isSecureContext: true, ...(captureMode === 'video' ? {} : { AudioContext: StubAudioContext }), addEventListener() {} },
    ...(captureMode === 'video' ? {} : { AudioContext: StubAudioContext }),
    navigator: { mediaDevices: { getUserMedia: async () => stream } }, performance, DOMException,
    setTimeout: (handler, delay) => setTimeout(handler, Math.min(delay, 5)), clearTimeout,
    fetch: async (_url, request) => { result = JSON.parse(request.body); return { ok: true }; },
  });
  new Script(pageScript()).runInContext(context);
  await handlers[buttonIds[captureMode]]();
  expect(stopped).toBe(tracks.length);
  expect(constructed).toBe(captureMode === 'video' ? 0 : 1);
  expect(closed).toBe(constructed);
  expect(result.capture_mode).toBe(captureMode);
  expect(result.status).toBe('failed');
  expect(result.reason).toBe('video_data_timeout');
  expect(result.error_name).toBe('TimeoutError');
  expect(result.all_tracks_ended).toBe(true);
  expect(result.audio_context_closed).toBe(true);
  expect(sanitizeMediaResult(result).status).toBe('failed');
  for (const id of Object.values(buttonIds)) expect(elements[id].disabled).toBe(false);
});

test('audio-only verifies its graph without creating a video and preserves a failed audio-clock result', async () => {
  const { handlers, elements } = pageElements();
  let result, stopped = 0, closed = 0;
  const track = { readyState: 'live', stop() { stopped++; this.readyState = 'ended'; } };
  const stream = { getTracks: () => [track], getAudioTracks: () => [track], getVideoTracks: () => [] };
  class StubAudioContext {
    state = 'running'; sampleRate = 48000; currentTime = 0;
    resume() { return Promise.resolve(); }
    createMediaStreamSource() { return { connect() {}, disconnect() {} }; }
    createAnalyser() { return { fftSize: 0, disconnect() {} }; }
    close() { this.state = 'closed'; closed++; return Promise.resolve(); }
  }
  const context = createContext({
    document: { hidden: false, getElementById: id => elements[id], addEventListener() {},
      createElement: () => { throw new Error('Audio-only must not create a video decoder'); } },
    window: { isSecureContext: true, AudioContext: StubAudioContext, addEventListener() {} }, AudioContext: StubAudioContext,
    navigator: { mediaDevices: { getUserMedia: async () => stream } }, performance, DOMException,
    setTimeout: (handler, delay) => setTimeout(handler, Math.min(delay, 5)), clearTimeout,
    fetch: async (_url, request) => { result = JSON.parse(request.body); return { ok: true }; },
  });
  new Script(pageScript()).runInContext(context);
  await handlers[buttonIds.audio]();
  expect(stopped).toBe(1);
  expect(closed).toBe(1);
  expect(result.capture_mode).toBe('audio');
  expect(result.status).toBe('failed');
  expect(result.reason).toBe('audio_clock_timeout');
  expect(result.error_name).toBe('TimeoutError');
  expect(result.video_tracks).toBe(0);
  expect(result.video_loaded_data).toBe(false);
  expect(result.audio_graph_connected).toBe(true);
  expect(result.all_tracks_ended).toBe(true);
  expect(result.audio_context_closed).toBe(true);
  expect(sanitizeMediaResult(result).status).toBe('failed');
});

test.skipIf(process.platform !== 'darwin')('isolates nonce HTTP, rejects invalid evidence, and preserves bounded independent denied attempts', async () => {
  const root = await realpath(await mkdtemp(join(tmpdir(), 'nomifun-media-contract-')));
  let fixture, child;
  try {
    // Exercise the actual CLI runtime. Bun's node:http compatibility layer can
    // return an empty 200 when its async request iterator rejects an oversized
    // body, while the real Node server correctly returns 400 without a report.
    child = spawn('node', [fileURLToPath(new URL('./global-os-media-fixture.mjs', import.meta.url)), '--report-dir', root], { stdio: ['ignore', 'pipe', 'pipe'] });
    const exit = new Promise(resolve => child.once('exit', (code, signal) => resolve({ code, signal })));
    let childError, stderr = '', startup;
    child.once('error', error => { childError = error; });
    child.stdout.resume();child.stderr.on('data', bytes => { stderr = (stderr + bytes).slice(-16_384); });
    // Read the private manifest, since Bun can omit external child stdout.
    const startupDeadline = Date.now() + 5000;
    while (!startup) {
      if (childError) throw childError;
      if (child.exitCode !== null) throw new Error(`Node fixture exited before startup (${child.exitCode}): ${stderr}`);
      const manifest = (await readdir(root)).find(name => /^media-server-[a-f0-9-]+\.json$/.test(name));
      if (manifest) {
        try { const metadata = JSON.parse(await readFile(join(root, manifest), 'utf8'));startup = { url: metadata.page_url, report: join(root, manifest) }; }
        catch (error) { if (!(error instanceof SyntaxError)) throw error; }
      }
      if (Date.now() > startupDeadline) throw new Error(`Node fixture startup deadline: ${stderr}`);
      if (!startup) await new Promise(resolve => setTimeout(resolve, 20));
    }
    let closing;
    fixture = { pageUrl: startup.url, origin: new URL(startup.url).origin, manifestPath: startup.report,
      close: () => closing ??= (async () => {
        child.kill('SIGINT');
        const endedProcess = await exit;
        expect(endedProcess).toEqual({ code: 0, signal: null });
        const endedFile = (await readdir(root)).find(name => name.startsWith('media-server-ended-'));
        expect(endedFile).toBeDefined();
        return join(root, endedFile);
      })() };
    const manifest = JSON.parse(await readFile(fixture.manifestPath, 'utf8'));
    expect(manifest.schema).toBe('nomifun.local-media-server.v2');
    expect(manifest.binding).toBe('127.0.0.1');
    expect(manifest.attempt_schema).toBe('nomifun.local-media-attempt.v2');
    expect(manifest.capture_modes).toEqual(captureModes);
    expect(fixture.pageUrl).toMatch(/^http:\/\/127\.0\.0\.1:\d+\/[a-f0-9]{48}\/$/);
    expect((await stat(root)).mode & 0o777).toBe(0o700);
    expect((await stat(fixture.manifestPath)).mode & 0o777).toBe(0o600);
    const page = await fetch(fixture.pageUrl);
    expect(page.status).toBe(200);
    const html = await page.text();
    const nonce = html.match(/<script nonce="([a-f0-9]{48})">/)[1];
    const csp = page.headers.get('content-security-policy');
    expect(csp).toContain("default-src 'none'");
    expect(csp).toContain(`script-src 'nonce-${nonce}'`);
    expect(csp).toContain(`style-src 'nonce-${nonce}'`);
    expect(csp).toContain("connect-src 'self'");
    expect(csp).toContain("frame-ancestors 'none'");
    expect(csp).not.toMatch(/unsafe-inline|unsafe-eval|https?:|\*/);
    expect(page.headers.get('permissions-policy')).toBe('camera=(self), microphone=(self), display-capture=(), geolocation=()');
    expect(page.headers.get('cache-control')).toBe('no-store');
    expect(page.headers.get('referrer-policy')).toBe('no-referrer');
    expect((await fetch(`${fixture.origin}/`)).status).toBe(404);
    expect((await fetch(`${fixture.origin}/wrong/result`, { method: 'POST', body: '{}' })).status).toBe(404);
    const post = (metadata, headers = {}) => fetch(`${fixture.pageUrl}result`, { method: 'POST',
      headers: { origin: fixture.origin, 'content-type': 'application/json', ...headers }, body: JSON.stringify(metadata) });
    expect((await post(deniedMetadata(), { origin: 'http://127.0.0.1:1' })).status).toBe(400);
    expect((await post(deniedMetadata(), { origin: '' })).status).toBe(400);
    expect((await post(deniedMetadata(), { 'content-type': 'text/plain' })).status).toBe(400);
    for (const captureMode of captureModes) {
      expect((await post({ ...deniedMetadata(1, captureMode), status: 'passed', reason: 'complete', error_name: undefined })).status).toBe(400);
    }
    const { capture_mode, error_constraint, ...legacy } = deniedMetadata();
    expect((await post({ ...legacy, schema: 'nomifun.local-media-attempt.v1' })).status).toBe(400);
    expect((await post({ ...deniedMetadata(), capture_mode: 'screen' })).status).toBe(400);
    expect((await post({ ...deniedMetadata(), error_constraint: 'private-device-identifier' })).status).toBe(400);
    expect((await post({ ...deniedMetadata(), samples: [] })).status).toBe(400);
    const oversized = await post({ ...deniedMetadata(), raw: 'x'.repeat(17_000) }).catch(() => undefined);
    expect(oversized?.status ?? 400).toBe(400);
    expect((await readdir(root)).filter(name => name.startsWith('media-attempt-'))).toEqual([]);
    // Concurrent requests exercise the post-body slot check. The only accepted
    // results are explicit synthetic failures in this disposable test directory.
    const results = await Promise.all(Array.from({ length: 40 }, (_, index) => post(deniedMetadata(index % 32 + 1, captureModes[index % captureModes.length]))));
    expect(results.filter(result => result.status === 200)).toHaveLength(32);
    expect(results.filter(result => result.status === 400)).toHaveLength(8);
    const reports = (await readdir(root)).filter(name => name.startsWith('media-attempt-'));
    expect(reports).toHaveLength(32);
    expect(new Set(reports).size).toBe(32);
    for (const name of reports) {
      const path = join(root, name);
      expect((await stat(path)).mode & 0o777).toBe(0o600);
      const report = JSON.parse(await readFile(path, 'utf8'));
      expect(report.schema).toBe('nomifun.local-media-attempt.v2');
      expect(captureModes).toContain(report.capture_mode);
      expect(report.error_constraint).toBe('');
      expect(report.status).toBe('failed');
      expect(report.error_name).toBe('NotAllowedError');
      expect(report.privacy).toEqual({ video_or_audio_content_retained: false, samples_read_or_retained: false, device_ids_or_labels_read: false });
      expect(report.scope).toContain('Self-reported');
    }
    const endPath = await fixture.close();
    const ended = JSON.parse(await readFile(endPath, 'utf8'));
    expect(ended.attempts_received).toBe(32);
    expect(ended.attempt_reports).toHaveLength(32);
    expect((await stat(endPath)).mode & 0o777).toBe(0o600);
  } finally {
    if (fixture) await fixture.close();else child?.kill('SIGINT');
    await rm(root, { recursive: true, force: true });
  }
}, 15000);
