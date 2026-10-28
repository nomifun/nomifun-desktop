import type { KeyboardSettings, KeyboardState } from '@/common/keyboard';

export type KeyboardSettingsAutosaveStatus = 'idle' | 'pending' | 'saving' | 'saved' | 'failed';
export interface KeyboardSettingsAutosaveSnapshot {
  draft: KeyboardSettings | null;
  saved: KeyboardSettings | null;
  status: KeyboardSettingsAutosaveStatus;
  pending: boolean;
  error: string | null;
  deferred: boolean;
}
export interface KeyboardSettingsAutosaveAdapter {
  updateSettings(settings: KeyboardSettings): Promise<KeyboardState>;
  getState(): Promise<KeyboardState>;
  listen(callback: (state: KeyboardState) => void): Promise<() => void>;
}
export interface KeyboardSettingsAutosaveScheduler {
  now(): number;
  setTimeout(callback: () => void, delayMs: number): unknown;
  clearTimeout(handle: unknown): void;
}
export interface KeyboardSettingsAutosaveController {
  getSnapshot(): KeyboardSettingsAutosaveSnapshot;
  subscribe(callback: () => void): () => void;
  attach(): () => void;
  observe(state: KeyboardState | null, captureBusy?: boolean): void;
  setDebounceMs(value: number): void;
  change(patch: Partial<KeyboardSettings>, mode?: 'immediate' | 'debounced'): void;
  flush(): Promise<boolean>;
  retry(): Promise<boolean>;
  compositionStart(field: keyof KeyboardSettings): void;
  compositionEnd<K extends keyof KeyboardSettings>(field: K, finalValue?: KeyboardSettings[K]): void;
}

type Field = keyof KeyboardSettings;
type Intent = { value: KeyboardSettings[Field]; version: number; due: number; held: boolean };
const recognitionFields = new Set<Field>(['asrMode', 'asrModel', 'audioDeviceId', 'language', 'dictionary']);
const defaultScheduler: KeyboardSettingsAutosaveScheduler = {
  now: () => Date.now(),
  setTimeout: (callback, delay) => setTimeout(callback, delay),
  clearTimeout: (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>),
};
const copy = <T,>(value: T): T => structuredClone(value);
const errorText = (failure: unknown): string => failure instanceof Error ? failure.message : String(failure);
const captureIsBusy = (state: KeyboardState | null): boolean => Boolean(state && (
  state.capturePreparing || state.phase !== 'idle'
  || ['starting', 'capturing', 'stopping'].includes(state.audioState.state)
  || !state.microphoneTest.stopped
  || ['starting', 'listening', 'stopping'].includes(state.microphoneTest.state)
));

/** Page-local intents survive navigation, but only a successful native write acknowledges them. */
export function createKeyboardSettingsAutosaveController(
  adapter: KeyboardSettingsAutosaveAdapter,
  options: { debounceMs?: number; scheduler?: KeyboardSettingsAutosaveScheduler } = {},
): KeyboardSettingsAutosaveController {
  const scheduler = options.scheduler ?? defaultScheduler;
  let debounceMs = options.debounceMs ?? 400;
  let native: KeyboardState | null = null;
  let observationRevision = 0;
  let saved: KeyboardSettings | null = null;
  let nextVersion = 0;
  const intents = new Map<Field, Intent>();
  const composing = new Map<Field, Intent | undefined>();
  const acknowledged = new Map<Field, number>();
  const listeners = new Set<() => void>();
  const consumers = new Set<symbol>();
  let awaitingMountedSnapshot = false;
  let externalCaptureBusy = false;
  let inFlight: Map<Field, Intent> | null = null;
  let error: string | null = null;
  let paused = false;
  let hasSaved = false;
  let timer: unknown;
  let backgroundEpoch = 0;
  let backgroundActive = false;
  let backgroundPending = false;
  let backgroundStop: (() => void) | undefined;
  let failureVersion = 0;
  const waiters = new Set<(success: boolean) => void>();
  let snapshot: KeyboardSettingsAutosaveSnapshot = {
    draft: null, saved: null, status: 'idle', pending: false, error: null, deferred: false,
  };

  const busy = () => externalCaptureBusy || captureIsBusy(native);
  const eligible = () => [...intents].filter(([field, intent]) =>
    !intent.held && intent.due <= scheduler.now() && !(recognitionFields.has(field) && busy()));
  const setField = (target: KeyboardSettings, field: Field, value: KeyboardSettings[Field]) => {
    Object.assign(target, { [field]: copy(value) });
  };
  const emit = () => {
    const draft = saved ? copy(saved) : null;
    if (draft) for (const [field, intent] of intents) setField(draft, field, intent.value);
    snapshot = {
      draft, saved: saved ? copy(saved) : null,
      status: inFlight ? 'saving' : paused ? 'failed' : intents.size ? 'pending' : hasSaved ? 'saved' : 'idle',
      pending: intents.size > 0 || inFlight !== null,
      error,
      deferred: busy() && [...intents.keys()].some(field => recognitionFields.has(field)),
    };
    for (const listener of listeners) listener();
  };
  const clearTimer = () => {
    if (timer !== undefined) scheduler.clearTimeout(timer);
    timer = undefined;
  };
  const stopBackground = () => {
    backgroundEpoch++;
    backgroundActive = false;
    backgroundPending = false;
    backgroundStop?.();
    backgroundStop = undefined;
  };
  const adopt = (next: KeyboardState, observation = true) => {
    if (native && next.sequence < native.sequence) return;
    if (observation) observationRevision++;
    native = copy(next);
    if (!saved) saved = copy(next.settings);
    else for (const field of Object.keys(next.settings) as Field[]) {
      // Persist may fail after native state has already echoed the requested value.
      if (!intents.has(field) && !inFlight?.has(field)) setField(saved, field, next.settings[field]);
    }
  };
  const settleWaiters = () => {
    if (inFlight || backgroundPending || (!paused && !awaitingMountedSnapshot && eligible().length)) return;
    const success = intents.size === 0 && !paused;
    for (const resolve of waiters) resolve(success);
    waiters.clear();
  };
  const fail = (failure: unknown) => {
    error = errorText(failure);
    failureVersion++;
    paused = true;
    clearTimer();
    emit();
    settleWaiters();
  };
  const ensureBackground = () => {
    if (consumers.size || backgroundActive || !intents.size || paused) return;
    backgroundActive = true;
    backgroundPending = true;
    const epoch = ++backgroundEpoch;
    const current = () => epoch === backgroundEpoch && consumers.size === 0;
    // Subscribe before reading: a capture can end while the initial read is in flight.
    void (async () => {
      try {
        const stop = await adapter.listen(next => {
          if (!current()) return;
          adopt(next);
          emit();
          pump();
        });
        if (!current()) { stop(); return; }
        backgroundStop = stop;
        const next = await adapter.getState();
        if (!current()) return;
        adopt(next);
        backgroundPending = false;
        emit();
        pump();
      } catch (failure) {
        if (!current()) return;
        stopBackground();
        fail(failure);
      }
    })();
  };
  const pump = () => {
    clearTimer();
    if (!intents.size && !inFlight) stopBackground();
    if (inFlight || paused || awaitingMountedSnapshot || backgroundPending) { settleWaiters(); return; }
    if (!native || !saved) {
      ensureBackground();
      settleWaiters();
      return;
    }
    const ready = eligible();
    if (!ready.length) {
      const due = [...intents].filter(([field, intent]) => !intent.held && !(recognitionFields.has(field) && busy()))
        .map(([, intent]) => intent.due);
      if (due.length) timer = scheduler.setTimeout(pump, Math.max(0, Math.min(...due) - scheduler.now()));
      if (!consumers.size) ensureBackground();
      settleWaiters();
      return;
    }
    const sent = new Map(ready.map(([field, intent]) => [field, { ...intent, value: copy(intent.value) }]));
    const payload = copy(native.settings);
    for (const [field, intent] of sent) setField(payload, field, intent.value);
    const startedAfterFailure = failureVersion;
    const startedAfterObservation = observationRevision;
    inFlight = sent;
    emit();
    void (async () => {
      try {
        const reply = await adapter.updateSettings(payload);
        const newerObservation = observationRevision !== startedAfterObservation;
        if (!native || reply.sequence > native.sequence || (reply.sequence === native.sequence && !newerObservation)) adopt(reply, false);
        for (const [field, intent] of sent) {
          // A newer native snapshot is the display/rebase authority even when this ACK is valid.
          const authoritative = native && (native.sequence > reply.sequence || (native.sequence === reply.sequence && newerObservation))
            ? native.settings : reply.settings;
          setField(saved!, field, authoritative[field]);
          acknowledged.set(field, intent.version);
          if (intents.get(field)?.version === intent.version) intents.delete(field);
        }
        hasSaved = true;
        if (failureVersion === startedAfterFailure) { error = null; paused = false; }
      } catch (failure) {
        fail(failure);
      } finally {
        inFlight = null;
        emit();
        pump();
      }
    })();
  };
  const flush = (): Promise<boolean> => {
    for (const intent of intents.values()) if (!intent.held) intent.due = scheduler.now();
    return new Promise(resolve => {
      waiters.add(resolve);
      if (!consumers.size) ensureBackground();
      pump();
    });
  };
  const change: KeyboardSettingsAutosaveController['change'] = (patch, mode = 'immediate') => {
    for (const field of Object.keys(patch) as Field[]) {
      const value = patch[field];
      if (value === undefined) continue;
      if (!intents.has(field) && saved && JSON.stringify(saved[field]) === JSON.stringify(value)) continue;
      intents.set(field, {
        value: copy(value), version: ++nextVersion,
        due: mode === 'debounced' ? scheduler.now() + debounceMs : scheduler.now(),
        held: composing.has(field),
      });
    }
    paused = false;
    emit();
    if (!consumers.size) ensureBackground();
    pump();
  };

  return {
    getSnapshot: () => snapshot,
    subscribe: (listener) => { listeners.add(listener); return () => { listeners.delete(listener); }; },
    attach: () => {
      const token = Symbol();
      if (!consumers.size) {
        stopBackground();
        awaitingMountedSnapshot = true;
      }
      consumers.add(token);
      return () => {
        if (!consumers.delete(token) || consumers.size) return;
        awaitingMountedSnapshot = false;
        externalCaptureBusy = false;
        for (const [field, prior] of composing) {
          if (prior && (acknowledged.get(field) ?? -1) < prior.version) intents.set(field, prior);
          else intents.delete(field);
        }
        composing.clear();
        emit();
        void flush();
      };
    },
    observe: (next, captureBusy = false) => {
      externalCaptureBusy = captureBusy;
      if (next) {
        adopt(next);
        awaitingMountedSnapshot = false;
      }
      emit();
      pump();
    },
    setDebounceMs: value => { debounceMs = value; },
    change,
    flush,
    retry: () => {
      paused = false;
      if (!consumers.size) { stopBackground(); ensureBackground(); }
      emit();
      return flush();
    },
    compositionStart: field => {
      if (!composing.has(field)) {
        const intent = intents.get(field);
        composing.set(field, intent ? copy(intent) : undefined);
      }
      const intent = intents.get(field);
      if (intent) intent.held = true;
      emit();
      pump();
    },
    compositionEnd: (field, finalValue) => {
      if (finalValue !== undefined) change({ [field]: finalValue }, 'debounced');
      composing.delete(field);
      const intent = intents.get(field);
      if (intent) { intent.held = false; intent.due = scheduler.now() + debounceMs; }
      emit();
      pump();
    },
  };
}
