import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { keyboard, documentAction, type KeyboardActionRequest, type KeyboardSelection, type KeyboardState } from '@/common/keyboard';

type Session = { id: string; document: string; revision: number; text: string; saved: string };
type Send = (request: KeyboardActionRequest) => Promise<KeyboardState | null>;
const editableState = (state: KeyboardState | null) => Boolean(state?.visible && state.phase === 'idle' && state.audioState.state === 'stopped');

/** The native lease fences hardware intents; the DOM owns the active caret/IME. */
export function useSubtitleEditor(state: KeyboardState | null, send: Send, getCurrent: () => KeyboardState | null,
  mayResume: () => boolean, beforeEnd: () => KeyboardSelection | null) {
  const session = useRef<Session | null>(null);
  const beginning = useRef<{ id: string; document: string | null; promise: Promise<boolean> } | null>(null);
  const flight = useRef<Promise<boolean> | null>(null);
  const sent = useRef<{ revision: number; text: string } | null>(null);
  const ending = useRef<Promise<boolean> | null>(null);
  const composing = useRef(false);
  const compositionWaiters = useRef<Array<() => void>>([]);
  const focused = useRef(false);
  const blocked = useRef(false);
  const alive = useRef(true);
  const uiAction = useRef<{ cancelGeneration: number | undefined } | null>(null);
  const epoch = useRef(0);
  const [version, setVersion] = useState(0);
  const [saveError, setSaveError] = useState(false);
  const [endingEdit, setEndingEdit] = useState(false);
  const update = useCallback(() => { if (alive.current) setVersion(value => value + 1); }, []);
  const fail = useCallback(() => { blocked.current = true; if (alive.current) setSaveError(true); update(); return false; }, [update]);
  const discard = useCallback(() => {
    epoch.current++;
    session.current = null; blocked.current = false; composing.current = false; focused.current = false;
    beginning.current = null; flight.current = null; sent.current = null; ending.current = null; uiAction.current = null;
    compositionWaiters.current.splice(0).forEach(resolve => resolve());
    if (alive.current) { setSaveError(false); setEndingEdit(false); }
    update();
  }, [update]);
  const cancelQueuedAction = useCallback(() => {
    uiAction.current = null;
    update();
  }, [update]);
  useLayoutEffect(() => {
    if (uiAction.current && uiAction.current.cancelGeneration !== state?.cancelGeneration) cancelQueuedAction();
  }, [state?.cancelGeneration, cancelQueuedAction]);

  const begin = useCallback(async () => {
    if (beginning.current) return beginning.current.promise;
    if (session.current || ending.current || uiAction.current) return Boolean(session.current);
    let current = getCurrent();
    if (!editableState(current) || !current) return false;
    const id = crypto.randomUUID();
    const generation = epoch.current;
    const promise = (async () => {
      if (current.editSessionId) {
        const cleared = await send({ ...documentAction(current, 'edit_cancel'), editId: current.editSessionId });
        const latest = getCurrent();
        if (!alive.current || epoch.current !== generation || !cleared || cleared.editSessionId !== null
          || latest?.documentId !== current.documentId || latest.editSessionId !== null || !editableState(latest)) return false;
        current = cleared;
      }
      const next = await send({ ...documentAction(current, 'edit_begin'), editId: id });
      const latest = getCurrent();
      if (!alive.current || epoch.current !== generation || !next || !next.documentId || next.editSessionId !== id
        || latest?.documentId !== next.documentId || latest.editSessionId !== id || !editableState(latest)) return false;
      session.current = { id, document: next.documentId, revision: next.revision, text: next.text, saved: next.text };
      blocked.current = false; setSaveError(false); update();
      return true;
    })();
    beginning.current = { id, document: current.documentId, promise };
    try { return await promise; } finally { if (beginning.current?.promise === promise) beginning.current = null; update(); }
  }, [getCurrent, send, update]);

  const pump = useCallback(async (): Promise<boolean> => {
    if (flight.current) return flight.current;
    if (composing.current || blocked.current) return false;
    const promise = (async () => {
      while (alive.current && session.current && !composing.current) {
        const draft = session.current;
        if (draft.text === draft.saved) return true;
        const current = getCurrent();
        if (!current || current.documentId !== draft.document || current.editSessionId !== draft.id
          || !editableState(current) || current.revision > draft.revision) return fail();
        const text = draft.text;
        const revision = draft.revision;
        const receipt = { revision, text };
        sent.current = receipt;
        const next = await send({ action: 'edit', documentId: draft.document, revision, editId: draft.id, text });
        if (sent.current === receipt) sent.current = null;
        if (!alive.current || session.current !== draft) return false;
        const latest = getCurrent();
        if (latest?.documentId !== draft.document || !latest.visible) { discard(); return false; }
        if (!next || next.documentId !== draft.document || next.editSessionId !== draft.id
          || next.revision !== revision + 1 || next.text !== text
          || latest.editSessionId !== draft.id || latest.revision > next.revision) return fail();
        draft.revision = next.revision; draft.saved = text;
        update();
      }
      return !composing.current && !blocked.current;
    })();
    flight.current = promise;
    update();
    try { return await promise; } finally { if (flight.current === promise) flight.current = null; update(); }
  }, [discard, fail, getCurrent, send, update]);

  const finish = useCallback(async (): Promise<boolean> => {
    if (ending.current) return ending.current;
    const generation = epoch.current;
    const promise = (async () => {
      if (beginning.current && !await beginning.current.promise) return false;
      for (;;) {
        if (composing.current) await new Promise<void>(resolve => compositionWaiters.current.push(resolve));
        if (!alive.current || epoch.current !== generation) return false;
        const saved = await pump();
        // An IME can start while the preceding input's native save is in flight.
        if (composing.current) continue;
        if (!saved) return false;
        break;
      }
      if (epoch.current !== generation) return false;
      const selection = beforeEnd();
      setEndingEdit(true);
      const draft = session.current;
      if (!draft) return !blocked.current;
      const current = getCurrent();
      if (!current || current.documentId !== draft.document || current.editSessionId !== draft.id || !editableState(current)) return fail();
      const next = await send({ action: 'edit_end', documentId: draft.document, revision: draft.revision, editId: draft.id, selection });
      if (!alive.current || session.current !== draft) return false;
      if (!next || next.editSessionId !== null) return fail();
      session.current = null; blocked.current = false; setSaveError(false); update();
      return true;
    })();
    ending.current = promise;
    try { return await promise; } finally {
      if (ending.current === promise) { ending.current = null; if (alive.current) setEndingEdit(false); }
      update();
    }
  }, [beforeEnd, fail, getCurrent, pump, send, update]);

  useEffect(() => {
    alive.current = true;
    let active = true;
    let dispose: (() => void) | undefined;
    if (keyboard.available()) void keyboard.listenEditFlush(request => {
      const draft = session.current;
      if ((draft?.document === request.documentId && draft.id === request.editId) || beginning.current?.id === request.editId) void finish();
    }).then(stop => { if (active) dispose = stop; else stop(); }).catch(() => { if (active) fail(); });
    return () => { active = false; alive.current = false; dispose?.(); compositionWaiters.current.splice(0).forEach(resolve => resolve()); };
  }, [fail, finish]);

  useLayoutEffect(() => {
    const pending = beginning.current;
    if (pending && state && (!state.visible || pending.document !== state.documentId
      && (pending.document !== null || state.editSessionId !== pending.id))) { discard(); return; }
    const draft = session.current;
    if (draft && state) {
      if (state.documentId !== draft.document || !state.visible) { discard(); return; }
      if (state.editSessionId !== draft.id || !editableState(state)) {
        if (draft.text !== draft.saved) fail();
        else if (!ending.current) { session.current = null; update(); }
      } else if (state.revision > draft.revision) {
        if (sent.current && state.revision === sent.current.revision + 1 && state.text === sent.current.text) {
          draft.revision = state.revision; draft.saved = state.text;
        } else if (state.text === draft.saved) draft.revision = state.revision;
        else fail();
      }
    }
    if (focused.current && mayResume() && !session.current && !beginning.current && !ending.current && !blocked.current && !uiAction.current
      && editableState(state) && !state?.editSessionId) void begin();
  }, [state, begin, discard, fail, mayResume, update]);

  const input = useCallback((text: string) => {
    const draft = session.current;
    if (!draft || !editableState(getCurrent()) || getCurrent()?.editSessionId !== draft.id) return;
    draft.text = text; update();
    if (!composing.current && !blocked.current) void pump();
  }, [getCurrent, pump, update]);
  const onFocus = useCallback(() => { focused.current = true; void begin(); }, [begin]);
  const onBlur = useCallback(() => { focused.current = false; void finish(); }, [finish]);
  const compositionStart = useCallback(() => { composing.current = true; update(); }, [update]);
  const compositionEnd = useCallback((text: string) => {
    composing.current = false; input(text);
    compositionWaiters.current.splice(0).forEach(resolve => resolve());
  }, [input]);
  const runAfterSave = useCallback(async (run: (current: KeyboardState) => Promise<KeyboardState | null>) => {
    const generation = epoch.current;
    const starting = getCurrent();
    const document = starting?.documentId;
    const cancellation = starting?.cancelGeneration;
    const operation = { cancelGeneration: cancellation };
    uiAction.current = operation;
    try {
      if (!await finish()) return;
      const current = getCurrent();
      if (current && epoch.current === generation && current.documentId === document
        && current.cancelGeneration === cancellation && uiAction.current === operation) await run(current);
    } finally { if (uiAction.current === operation) uiAction.current = null; update(); }
  }, [finish, getCurrent, update]);
  const retry = useCallback(async () => {
    const generation = epoch.current;
    const draft = session.current;
    const current = getCurrent();
    if (!draft || !current || current.documentId !== draft.document || !editableState(current)) return;
    const text = draft.text;
    if (current.editSessionId !== draft.id) {
      session.current = null;
      const acquired = await begin();
      const resumed = session.current as Session | null;
      if (epoch.current !== generation || getCurrent()?.documentId !== draft.document) return;
      if (!acquired || !resumed) { session.current = draft; fail(); return; }
      resumed.text = text;
    } else draft.revision = current.revision;
    blocked.current = false; setSaveError(false);
    if (await pump()) await finish();
  }, [begin, fail, finish, getCurrent, pump]);
  const draft = session.current;
  void version;
  return {
    text: draft && draft.document === state?.documentId ? draft.text : null,
    editable: Boolean(draft && state?.editSessionId === draft.id && editableState(state) && !endingEdit),
    saving: Boolean(flight.current), composing: composing.current, saveError, onFocus, onBlur, input, compositionStart, compositionEnd,
    runAfterSave, cancelQueuedAction, discard, retry,
  };
}
