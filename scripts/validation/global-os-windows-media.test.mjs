// Pure protocol fixtures: no WebView, permission, device or media samples.
import assert from 'node:assert/strict';
import test from 'node:test';
import vm from 'node:vm';
import { startWindowsNativeMediaProbe, cancelWindowsNativeMediaProbe } from './global-os-windows-media.mjs';

const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
const tick = () => new Promise(done => setTimeout(done, 0));
async function until(predicate) { for (let at = 0; at < 100; at++) { if (predicate()) return; await tick(); } assert.fail('Protocol fixture did not reach its bounded checkpoint'); }
function fixture({ delayedAcquisition, delayedClose, delayedResume, disconnectFailure = false, constructorFailure = false, closeFailure = false, stopFailure = false } = {}) {
  const stats = { acquisitions: 0, closes: 0, source_disconnect: 0, gain_disconnect: 0, samples_read: 0 };
  const tracks = [0, 1].map(index => ({ kind: 'audio', readyState: 'live', enabled: true, muted: false,
    getSettings: () => ({ sampleRate: 48000, channelCount: 1 }), stop() { if (stopFailure && index === 0) throw new Error('fixture stop failure'); this.readyState = 'ended'; },
  }));
  const stream = { getTracks: () => tracks };
  let context, clock = 0;
  class FakeContext {
    constructor() { if (constructorFailure) throw new Error('fixture constructor failure'); context = this; this.state = 'suspended'; this.destination = {}; }
    get currentTime() { return ++clock; }
    createMediaStreamSource() { return { connect() {}, disconnect() { stats.source_disconnect++; if (disconnectFailure) throw new Error('fixture disconnect failure'); } }; }
    createGain() { return { gain: { value: 1 }, connect() {}, disconnect() { stats.gain_disconnect++; } }; }
    createAnalyser() { stats.samples_read++; throw new Error('Sample access forbidden'); }
    createScriptProcessor() { stats.samples_read++; throw new Error('Sample access forbidden'); }
    async resume() { if (delayedResume) await delayedResume.promise; if (this.state !== 'closed') this.state = 'running'; }
    async close() { stats.closes++; if (delayedClose) await delayedClose.promise; if (closeFailure) throw new Error('fixture close failure'); this.state = 'closed'; }
  }
  const window = {};
  const world = vm.createContext({ window, AudioContext: FakeContext, performance: { now: () => ++clock },
    setTimeout: (done, ms) => setTimeout(done, ms === 250 ? 0 : ms), clearTimeout,
    navigator: { mediaDevices: { getUserMedia: () => { stats.acquisitions++; return delayedAcquisition?.promise ?? Promise.resolve(stream); } } },
  });
  const key = '__pure_protocol_fixture';
  vm.runInContext(`(${startWindowsNativeMediaProbe.toString()})(${JSON.stringify(key)},'audio')`, world);
  return { state: window[key], stats, tracks, stream, context: () => context,
    cancel: ms => vm.runInContext(`(${cancelWindowsNativeMediaProbe.toString()})(${JSON.stringify(key)},${ms})`, world) };
}

test('media terminal status waits for the actual context close promise', async () => {
  const close = deferred(), value = fixture({ delayedClose: close });
  await until(() => value.stats.closes === 1);
  assert.equal(value.state.status, 'pending'); assert.equal(value.state.cleanup_proven, false);
  assert(value.tracks.every(track => track.readyState === 'ended'));
  close.resolve(); await value.state.pending;
  assert.equal(value.state.status, 'acquired'); assert.equal(value.state.cleanup_proven, true); assert.equal(value.state.context_closed, true);
  assert.equal(value.stats.samples_read, 0);
});

test('disconnect failure still attempts every other cleanup and context close', async () => {
  const value = fixture({ disconnectFailure: true }); await value.state.pending;
  assert.equal(value.stats.source_disconnect, 1); assert.equal(value.stats.gain_disconnect, 1); assert.equal(value.stats.closes, 1);
  assert(value.state.cleanup_errors.some(error => error.stage === 'source.disconnect'));
  assert.equal(value.state.status, 'failed'); assert.equal(value.state.capture_status, 'acquired'); assert.equal(value.state.cleanup_proven, true);
});

test('context constructor failure proves context absence and stops acquired tracks', async () => {
  const value = fixture({ constructorFailure: true }); await value.state.pending;
  assert.equal(value.state.context_created, false); assert.equal(value.stats.closes, 0);
  assert.equal(value.state.status, 'failed'); assert.equal(value.state.all_tracks_ended, true); assert.equal(value.state.cleanup_proven, true);
});

test('unresolved permission acquisition stays unknown; a late stream joins the same cancelled request', async () => {
  const acquisition = deferred(), value = fixture({ delayedAcquisition: acquisition }), original = value.state.pending;
  const unresolved = await value.cancel(1);
  assert.equal(unresolved.joined, false); assert.equal(unresolved.status, 'pending'); assert.equal(unresolved.cleanup_proven, false);
  assert.equal(unresolved.context_created, false); assert.equal(value.stats.acquisitions, 1);
  acquisition.resolve(value.stream); await original;
  assert.equal(value.state.pending, original); assert.equal(value.state.status, 'cancelled'); assert.equal(value.state.cleanup_proven, true);
  assert(value.tracks.every(track => track.readyState === 'ended')); assert.equal(value.stats.closes, 0); assert.equal(value.stats.acquisitions, 1);
});

test('cancellation during resume closes once and joins without another capture', async () => {
  const resume = deferred(), value = fixture({ delayedResume: resume }); await until(() => value.context());
  const original = value.state.pending, result = await value.cancel(100);
  assert.equal(result.status, 'cancelled'); assert.equal(result.joined, true); assert.equal(result.cleanup_proven, true);
  assert.equal(value.state.pending, original); assert.equal(value.stats.acquisitions, 1); assert.equal(value.stats.closes, 1);
  assert.equal(value.stats.samples_read, 0); resume.resolve(); await tick(); assert.equal(value.context().state, 'closed');
});

test('a rejected close cannot become cleanup proof or trigger an automatic new close', async () => {
  const value = fixture({ closeFailure: true }); await value.state.pending;
  assert.equal(value.state.status, 'cleanup_unproven'); assert.equal(value.state.cleanup_proven, false); assert.equal(value.state.context_closed, false);
  const result = await value.cancel(100); assert.equal(result.cleanup_proven, false); assert.equal(value.stats.closes, 1);
});

test('one failed track stop does not skip other stops or closing the context', async () => {
  const value = fixture({ stopFailure: true }); await value.state.pending;
  assert.equal(value.tracks[1].readyState, 'ended'); assert.equal(value.stats.closes, 1);
  assert.equal(value.state.all_tracks_ended, false); assert.equal(value.state.cleanup_proven, false); assert.equal(value.state.status, 'cleanup_unproven');
});

test('an empty returned stream cannot masquerade as capture success', async () => {
  const acquisition = deferred(), value = fixture({ delayedAcquisition: acquisition });
  acquisition.resolve({ getTracks: () => [] }); await value.state.pending;
  assert.equal(value.state.status, 'failed'); assert.equal(value.state.capture_status, 'failed');
  assert.equal(value.state.context_created, false); assert.equal(value.state.cleanup_proven, true);
  assert.match(value.state.error.message, /live requested tracks/);
});
