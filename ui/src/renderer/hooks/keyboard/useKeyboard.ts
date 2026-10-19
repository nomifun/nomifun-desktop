import { useCallback, useEffect, useRef, useState } from 'react';
import { keyboard, type KeyboardActionRequest, type KeyboardSettings, type KeyboardState } from '@/common/keyboard';

const errorText = (error: unknown) => error instanceof Error ? error.message : String(error);

/** Subscribe before reading so window recreation cannot miss a native state transition. */
export function useKeyboard() {
  const [state, setState] = useState<KeyboardState | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const mounted = useRef(true);
  const events = useRef(0);
  const requests = useRef(0);
  const mutations = useRef(0);
  const current = useRef<KeyboardState | null>(null);

  const request = useCallback(async (run: () => Promise<KeyboardState>, mutation = false) => {
    const generation = events.current;
    const requestId = ++requests.current;
    if (mutation) { mutations.current++; setBusy(true); }
    setError(null);
    try {
      const next = await run();
      const currentSequence = current.current?.sequence ?? -1;
      if (mounted.current && (next.sequence > currentSequence || (next.sequence === currentSequence
        && generation === events.current && requestId === requests.current))) {
        current.current = next;
        setState(next);
      }
      return next;
    } catch (failure) {
      if (mounted.current && generation === events.current && requestId === requests.current) setError(errorText(failure));
      return null;
    } finally {
      if (mutation) {
        mutations.current--;
        if (mounted.current) setBusy(mutations.current > 0);
      }
    }
  }, []);

  useEffect(() => {
    mounted.current = true;
    if (!keyboard.available()) return () => { mounted.current = false; };
    let active = true;
    let dispose: (() => void) | undefined;
    void keyboard.listen((next) => {
      if (!active) return;
      if (next.sequence < (current.current?.sequence ?? -1)) return;
      events.current++;
      current.current = next;
      setState(next);
    }).then((stop) => {
      if (!active) { stop(); return; }
      dispose = stop;
      void request(keyboard.getState);
    }).catch((failure: unknown) => { if (active) setError(errorText(failure)); });
    return () => { active = false; mounted.current = false; dispose?.(); };
  }, [request]);

  const refresh = useCallback(() => request(keyboard.getState), [request]);
  const updateSettings = useCallback((settings: KeyboardSettings) => request(() => keyboard.updateSettings(settings), true), [request]);
  const action = useCallback((value: KeyboardActionRequest) => request(() => keyboard.action(value), true), [request]);
  const getCurrentState = useCallback(() => current.current, []);
  return { state, error, busy, available: keyboard.available(), refresh, updateSettings, action, getCurrentState };
}
