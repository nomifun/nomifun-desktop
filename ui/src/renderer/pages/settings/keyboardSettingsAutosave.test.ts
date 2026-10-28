import { describe, expect, test } from 'bun:test';
import type { KeyboardSettings, KeyboardState } from '@/common/keyboard';
import {
  createKeyboardSettingsAutosaveController,
  type KeyboardSettingsAutosaveAdapter,
  type KeyboardSettingsAutosaveScheduler,
} from './keyboardSettingsAutosave';

function state(sequence = 1, patch: Partial<KeyboardSettings> = {}): KeyboardState {
  return {
    sequence, cancelGeneration: 0,
    settings: {
      enabled: true, k2Enabled: true, k3Enabled: true, k4Enabled: true,
      audioDeviceId: null, asrMode: 'streaming', asrModel: null, textModel: null,
      language: '', dictionary: [], beautifyInstruction: '', alwaysOnTop: false,
      subtitleFontSize: 14, ...patch,
    },
    documentId: null, revision: 0, operationId: null, editSessionId: null,
    dictationSelection: null, phase: 'idle', text: '', originalText: '', partial: '',
    candidate: '', visible: false, pendingAction: null, error: null, notice: null,
    capturePreparing: false, audioState: { state: 'stopped', detail: null },
    microphoneTest: {
      id: null, deviceId: null, state: 'idle', callbackObserved: false, samples: 0,
      nonzero: 0, peak: 0, rms: 0, elapsedMs: 0, stopped: true, detail: null,
    },
    device: { connected: false, id: null, port: null, firmware: null, policyState: 'pending', captureState: 'stopped', detail: null },
    service: { running: false, ready: false, detail: null },
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

class Clock implements KeyboardSettingsAutosaveScheduler {
  time = 0;
  private next = 0;
  private timers = new Map<number, { at: number; run: () => void }>();
  now = () => this.time;
  setTimeout = (run: () => void, delay: number) => {
    const id = ++this.next;
    this.timers.set(id, { at: this.time + delay, run });
    return id;
  };
  clearTimeout = (id: unknown) => { this.timers.delete(id as number); };
  advance(ms: number) {
    const end = this.time + ms;
    for (;;) {
      const next = [...this.timers].filter(([, timer]) => timer.at <= end).sort((a, b) => a[1].at - b[1].at)[0];
      if (!next) break;
      this.time = next[1].at;
      this.timers.delete(next[0]);
      next[1].run();
    }
    this.time = end;
  }
}

const settle = async () => { for (let index = 0; index < 8; index++) await Promise.resolve(); };

function fixture() {
  const clock = new Clock();
  const writes: Array<{ settings: KeyboardSettings; reply: ReturnType<typeof deferred<KeyboardState>> }> = [];
  const listeners = new Set<(next: KeyboardState) => void>();
  let current = state();
  let stops = 0;
  const adapter: KeyboardSettingsAutosaveAdapter = {
    updateSettings: (settings) => {
      const reply = deferred<KeyboardState>();
      writes.push({ settings: structuredClone(settings), reply });
      return reply.promise;
    },
    getState: async () => current,
    listen: async (callback) => {
      listeners.add(callback);
      return () => { listeners.delete(callback); stops++; };
    },
  };
  const controller = createKeyboardSettingsAutosaveController(adapter, { scheduler: clock });
  const detach = controller.attach();
  controller.observe(current);
  const acknowledge = async (index: number, sequence = index + 2) => {
    current = state(sequence, writes[index].settings);
    writes[index].reply.resolve(current);
    await settle();
  };
  const publish = (next: KeyboardState) => {
    current = next;
    for (const callback of listeners) callback(next);
  };
  return { controller, clock, writes, adapter, listeners, detach, acknowledge, publish, stops: () => stops };
}

describe('keyboard settings autosave controller', () => {
  test('observing mount and unrelated native events never writes settings', () => {
    const f = fixture();
    f.controller.observe({ ...state(2), text: 'An unrelated transcript update' });
    expect(f.writes).toHaveLength(0);
    expect(f.controller.getSnapshot()).toMatchObject({ status: 'idle', pending: false, error: null, deferred: false });
  });

  test('serializes rapid selectors and switches and merges only after the first ACK', async () => {
    const f = fixture();
    f.controller.change({ asrMode: 'platform', asrModel: { providerId: 'p', model: 'file' } });
    f.controller.change({ k4Enabled: false });
    f.controller.change({ alwaysOnTop: true });
    expect(f.writes).toHaveLength(1);
    expect(f.writes[0].settings).toMatchObject({ asrMode: 'platform', asrModel: { providerId: 'p', model: 'file' }, k4Enabled: true });
    await f.acknowledge(0);
    expect(f.writes).toHaveLength(2);
    expect(f.writes[1].settings).toMatchObject({ asrMode: 'platform', k4Enabled: false, alwaysOnTop: true });
    await f.acknowledge(1);
    expect(f.controller.getSnapshot()).toMatchObject({ status: 'saved', pending: false });
  });

  test('field versions preserve ABA input even when an older ACK has the same value', async () => {
    const f = fixture();
    f.controller.change({ k4Enabled: false });
    f.controller.change({ k4Enabled: true });
    f.controller.change({ k4Enabled: false });
    await f.acknowledge(0);
    expect(f.controller.getSnapshot().draft?.k4Enabled).toBe(false);
    expect(f.controller.getSnapshot().pending).toBe(true);
    expect(f.writes).toHaveLength(2);
    await f.acknowledge(1);
    expect(f.controller.getSnapshot().pending).toBe(false);
  });

  test('rebases queued edits on newer native settings and rejects an older ACK as a base', async () => {
    const f = fixture();
    f.controller.change({ k4Enabled: false });
    f.controller.change({ k3Enabled: false });
    f.controller.observe(state(10, { alwaysOnTop: true, k4Enabled: false }));
    await f.acknowledge(0, 2);
    expect(f.controller.getSnapshot().draft?.alwaysOnTop).toBe(true);
    expect(f.writes[1].settings).toMatchObject({ alwaysOnTop: true, k3Enabled: false });
    await f.acknowledge(1, 11);
  });

  test('a same-sequence ACK cannot replace a native snapshot received after dispatch', async () => {
    const f = fixture();
    f.controller.change({ k4Enabled: false });
    f.controller.change({ k3Enabled: false });
    f.controller.observe(state(1, { alwaysOnTop: true, k4Enabled: false }));
    await f.acknowledge(0, 1);
    expect(f.controller.getSnapshot().draft?.alwaysOnTop).toBe(true);
    expect(f.writes[1].settings.alwaysOnTop).toBe(true);
    await f.acknowledge(1, 2);
  });

  test('successful normalization replaces the sent intent without a repeat write', async () => {
    const f = fixture();
    f.controller.change({ dictionary: ['  NomiFun  '] });
    f.writes[0].reply.resolve(state(2, { dictionary: ['NomiFun'] }));
    await settle();
    expect(f.controller.getSnapshot()).toMatchObject({ pending: false, draft: { dictionary: ['NomiFun'] } });
    f.clock.advance(1_000);
    expect(f.writes).toHaveLength(1);
  });

  test('echoed native values never acknowledge a failed persist; retry rebases on current native state', async () => {
    const f = fixture();
    f.controller.change({ language: 'zh' });
    f.controller.observe(state(2, { language: 'zh' }));
    f.writes[0].reply.reject(new Error('Disk unavailable'));
    await settle();
    expect(f.controller.getSnapshot()).toMatchObject({ status: 'failed', pending: true, error: 'Disk unavailable' });
    expect(f.controller.getSnapshot().saved?.language).toBe('');
    f.controller.observe(state(3, { language: 'zh', alwaysOnTop: true }));
    expect(f.controller.getSnapshot().error).toBe('Disk unavailable');
    expect(f.writes).toHaveLength(1);
    const retry = f.controller.retry();
    expect(f.writes[1].settings).toMatchObject({ language: 'zh', alwaysOnTop: true });
    await f.acknowledge(1, 4);
    expect(await retry).toBe(true);
    expect(f.controller.getSnapshot()).toMatchObject({ status: 'saved', pending: false, error: null });
  });

  test('a new edit retries a failure once while native events alone do not retry', async () => {
    const f = fixture();
    f.controller.change({ k4Enabled: false });
    f.writes[0].reply.reject(new Error('Write failed'));
    await settle();
    f.controller.observe(state(2));
    expect(f.writes).toHaveLength(1);
    f.controller.change({ alwaysOnTop: true });
    expect(f.writes).toHaveLength(2);
    expect(f.writes[1].settings).toMatchObject({ k4Enabled: false, alwaysOnTop: true });
    f.writes[1].reply.reject(new Error('Still unavailable'));
    await settle();
    f.clock.advance(5_000);
    expect(f.writes).toHaveLength(2);
    expect(f.controller.getSnapshot().status).toBe('failed');
  });

  test('a safe save during capture preserves native language echoed before an earlier persist failure', async () => {
    const f = fixture();
    f.controller.change({ language: 'zh' });
    f.controller.observe(state(2, { language: 'zh' }));
    f.writes[0].reply.reject(new Error('Disk unavailable'));
    await settle();
    expect(f.controller.getSnapshot()).toMatchObject({ status: 'failed', pending: true, saved: { language: '' } });
    f.controller.observe({
      ...state(3, { language: 'zh' }), phase: 'recording',
      audioState: { state: 'capturing', detail: null },
    });
    f.controller.change({ subtitleFontSize: 18, alwaysOnTop: true });
    expect(f.writes).toHaveLength(2);
    expect(f.writes[1].settings).toMatchObject({ language: 'zh', subtitleFontSize: 18, alwaysOnTop: true });
    f.writes[1].reply.resolve({
      ...state(4, f.writes[1].settings), phase: 'recording',
      audioState: { state: 'capturing', detail: null },
    });
    await settle();
    expect(f.controller.getSnapshot()).toMatchObject({ pending: true, deferred: true, saved: { language: '' }, draft: { language: 'zh' } });
    expect(f.writes).toHaveLength(2);
  });

  test('a debounced deadline survives a different field write in flight', async () => {
    const f = fixture();
    f.controller.change({ k4Enabled: false });
    f.clock.advance(100);
    f.controller.change({ language: 'en' }, 'debounced');
    f.clock.advance(100);
    await f.acknowledge(0);
    expect(f.writes).toHaveLength(1);
    f.clock.advance(299);
    expect(f.writes).toHaveLength(1);
    f.clock.advance(1);
    expect(f.writes[1].settings.language).toBe('en');
    await f.acknowledge(1);
  });

  test('blur flush sends existing text but preserves the deadline of a newer edit during the write', async () => {
    const f = fixture();
    f.controller.change({ language: 'en' }, 'debounced');
    const flush = f.controller.flush();
    expect(f.writes).toHaveLength(1);
    f.clock.advance(100);
    f.controller.change({ language: 'zh' }, 'debounced');
    await f.acknowledge(0);
    expect(await flush).toBe(false);
    expect(f.writes).toHaveLength(1);
    expect(f.controller.getSnapshot().draft?.language).toBe('zh');
    f.clock.advance(400);
    expect(f.writes[1].settings.language).toBe('zh');
    await f.acknowledge(1);
  });

  test('IME text cannot leak through a safe switch or blur and waits 400 ms after composition end', async () => {
    const f = fixture();
    f.controller.compositionStart('language');
    f.controller.change({ language: 'zh-Ha' }, 'debounced');
    f.controller.change({ k4Enabled: false });
    expect(f.writes[0].settings).toMatchObject({ language: '', k4Enabled: false });
    const flush = f.controller.flush();
    await f.acknowledge(0);
    expect(await flush).toBe(false);
    f.clock.advance(1_000);
    expect(f.writes).toHaveLength(1);
    f.controller.change({ language: 'zh-Hans' }, 'debounced');
    f.controller.compositionEnd('language');
    f.clock.advance(399);
    expect(f.writes).toHaveLength(1);
    f.clock.advance(1);
    expect(f.writes[1].settings.language).toBe('zh-Hans');
    await f.acknowledge(1);
  });

  test('recognition fields defer during capture while a disable switch saves without them', async () => {
    const f = fixture();
    f.controller.observe({ ...state(2), capturePreparing: true });
    f.controller.change({ language: 'zh', dictionary: ['NomiFun'], audioDeviceId: 'mic-2' }, 'debounced');
    f.controller.change({ enabled: false });
    expect(f.writes[0].settings).toMatchObject({ enabled: false, language: '', dictionary: [], audioDeviceId: null });
    expect(f.controller.getSnapshot().deferred).toBe(true);
    f.clock.advance(500);
    f.writes[0].reply.resolve({ ...state(3, f.writes[0].settings), capturePreparing: true });
    await settle();
    expect(f.writes).toHaveLength(1);
    f.controller.observe(state(5, { enabled: false }));
    expect(f.writes[1].settings).toMatchObject({ enabled: false, language: 'zh', dictionary: ['NomiFun'], audioDeviceId: 'mic-2' });
    await f.acknowledge(1, 6);
  });

  test('a safe update never includes a debounced field that has not reached its deadline', async () => {
    const f = fixture();
    f.controller.change({ dictionary: ['Pending word'] }, 'debounced');
    f.controller.change({ k4Enabled: false });
    expect(f.writes[0].settings.dictionary).toEqual([]);
    await f.acknowledge(0);
    expect(f.writes).toHaveLength(1);
    f.clock.advance(400);
    expect(f.writes[1].settings.dictionary).toEqual(['Pending word']);
    await f.acknowledge(1);
  });

  test('detached pending capture edits resume from a temporary native subscription and release it', async () => {
    const f = fixture();
    const busy = { ...state(2), audioState: { state: 'capturing' as const, detail: null } };
    f.publish(busy);
    f.controller.observe(busy);
    f.controller.change({ language: 'zh' }, 'debounced');
    f.detach();
    await settle();
    expect(f.writes).toHaveLength(0);
    expect(f.listeners.size).toBe(1);
    f.publish(state(3, { alwaysOnTop: true }));
    expect(f.writes[0].settings).toMatchObject({ language: 'zh', alwaysOnTop: true });
    await f.acknowledge(0, 4);
    expect(f.listeners.size).toBe(0);
    expect(f.stops()).toBe(1);
  });

  test('unmount flush and remount preserve new input while the older write is still in flight', async () => {
    const f = fixture();
    f.controller.change({ k4Enabled: false });
    f.controller.change({ language: 'en' }, 'debounced');
    f.detach();
    await settle();
    const detachAgain = f.controller.attach();
    f.controller.observe(state(2));
    f.controller.change({ language: 'zh' }, 'debounced');
    await f.acknowledge(0, 3);
    expect(f.controller.getSnapshot().draft?.language).toBe('zh');
    expect(f.writes).toHaveLength(1);
    f.clock.advance(400);
    expect(f.writes[1].settings.language).toBe('zh');
    await f.acknowledge(1, 4);
    detachAgain();
  });

  test('unmount discards unfinished composition without writing it or retaining a listener', async () => {
    const f = fixture();
    f.controller.compositionStart('language');
    f.controller.change({ language: 'half' }, 'debounced');
    f.detach();
    await settle();
    f.clock.advance(1_000);
    expect(f.writes).toHaveLength(0);
    expect(f.listeners.size).toBe(0);
    expect(f.controller.getSnapshot()).toMatchObject({ pending: false, draft: { language: '' } });
    const detachAgain = f.controller.attach();
    f.controller.observe(state(2));
    expect(await f.controller.flush()).toBe(true);
    f.controller.change({ language: 'complete' }, 'debounced');
    f.clock.advance(400);
    expect(f.writes[0].settings.language).toBe('complete');
    await f.acknowledge(0, 3);
    detachAgain();
  });

  test('unmount restores and flushes a complete intent that preceded composition', async () => {
    const f = fixture();
    f.controller.change({ language: 'en' }, 'debounced');
    f.controller.compositionStart('language');
    f.controller.change({ language: 'half' }, 'debounced');
    f.detach();
    await settle();
    expect(f.writes[0].settings.language).toBe('en');
    await f.acknowledge(0);
    expect(f.listeners.size).toBe(0);
  });

  test('composition end can atomically accept the final native input value', async () => {
    const f = fixture();
    f.controller.compositionStart('dictionary');
    f.controller.change({ dictionary: ['intermediate'] }, 'debounced');
    f.controller.compositionEnd('dictionary', ['Final term']);
    f.clock.advance(400);
    expect(f.writes[0].settings.dictionary).toEqual(['Final term']);
    await f.acknowledge(0);
  });

  test.each(['listen', 'getState'] as const)('a detached %s failure remains visible and can recover on retry', async (method) => {
    const f = fixture();
    const busy = { ...state(2), capturePreparing: true };
    f.publish(busy);
    f.controller.observe(busy);
    f.controller.change({ language: 'zh' }, 'debounced');
    const original = f.adapter[method];
    f.adapter[method] = async () => { throw new Error('Native state unavailable'); };
    f.detach();
    await settle();
    expect(f.controller.getSnapshot()).toMatchObject({ status: 'failed', pending: true, error: 'Native state unavailable' });
    expect(f.writes).toHaveLength(0);
    Object.assign(f.adapter, { [method]: original });
    f.publish(state(3));
    const retry = f.controller.retry();
    await settle();
    expect(f.writes[0].settings.language).toBe('zh');
    await f.acknowledge(0, 4);
    expect(await retry).toBe(true);
    expect(f.controller.getSnapshot().error).toBeNull();
  });

  test('a late detached listener is immediately released after remount and cannot publish stale failure', async () => {
    const f = fixture();
    const subscription = deferred<() => void>();
    let staleCallback: ((next: KeyboardState) => void) | undefined;
    let stopped = 0;
    f.adapter.listen = callback => { staleCallback = callback; return subscription.promise; };
    f.controller.change({ language: 'en' }, 'debounced');
    f.detach();
    const detachAgain = f.controller.attach();
    f.controller.observe(state(5, { alwaysOnTop: true }));
    f.controller.change({ language: 'zh' }, 'debounced');
    subscription.resolve(() => { stopped++; });
    await settle();
    expect(stopped).toBe(1);
    staleCallback?.(state(2));
    expect(f.controller.getSnapshot().draft?.alwaysOnTop).toBe(true);
    expect(f.controller.getSnapshot().draft?.language).toBe('zh');
    // A freshly observed mount may already have dispatched the previous complete intent.
    await f.acknowledge(0, 6);
    expect(f.controller.getSnapshot().draft?.language).toBe('zh');
    f.clock.advance(400);
    expect(f.writes[1].settings).toMatchObject({ language: 'zh', alwaysOnTop: true });
    await f.acknowledge(1, 7);
    detachAgain();
  });
});
