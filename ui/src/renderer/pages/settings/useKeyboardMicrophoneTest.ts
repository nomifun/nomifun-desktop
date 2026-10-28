import { useCallback, useEffect, useRef, useState } from 'react';
import { keyboard, type KeyboardMicrophoneTest, type KeyboardState } from '@/common/keyboard';

type Owner = {
  id: string;
  deviceId: string | null;
  sequence: number;
  disposed: boolean;
  stopRequested: boolean;
  stopped: boolean;
  startSettled: boolean;
  noOwnerStopSequence: number | null;
};
const idle = (): KeyboardMicrophoneTest => ({ id: null, deviceId: null, state: 'idle', callbackObserved: false,
  samples: 0, nonzero: 0, peak: 0, rms: 0, elapsedMs: 0, stopped: true, detail: null });
const stopOwned = (id: string) => keyboard.action({ action: 'microphone_test_stop', testId: id });

/** A page owns exactly the native check it started, including before start acknowledges. */
export function useKeyboardMicrophoneTest(state: KeyboardState | null, deviceId: string | null) {
  const owner = useRef<Owner | null>(null);
  const mounted = useRef(true);
  const [result, setResult] = useState<KeyboardMicrophoneTest>(idle);
  const observe = useCallback((next: KeyboardState) => {
    const request = owner.current;
    if (!mounted.current || !request || request.disposed || next.sequence < request.sequence || next.microphoneTest.id !== request.id) return;
    request.sequence = next.sequence;
    request.stopped = next.microphoneTest.stopped;
    setResult(request.stopRequested && !next.microphoneTest.stopped
      ? { ...next.microphoneTest, state: 'stopping' } : next.microphoneTest);
  }, []);
  const observeStop = useCallback((next: KeyboardState, request: Owner) => {
    if (!mounted.current || owner.current !== request || request.disposed || next.sequence < request.sequence) return;
    if (next.microphoneTest.id === request.id) { observe(next); return; }
    // This exact stop was accepted while the native slot belongs to no such ID.
    // A pending start still needs its own late-ack stop before the UI can finish.
    request.noOwnerStopSequence = next.sequence;
    request.sequence = next.sequence;
    if (!request.startSettled) return;
    request.stopped = true;
    setResult(previous => ({ ...previous, stopped: true, state: previous.state === 'failed' ? 'failed' : 'cancelled' }));
  }, [observe]);
  useEffect(() => { if (state) observe(state); }, [observe, state]);
  const dispose = useCallback(() => {
    const request = owner.current;
    if (!request) return;
    request.disposed = true;
    owner.current = null;
    void stopOwned(request.id).catch(() => { /* Native test is bounded even if the transport closes. */ });
  }, []);
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; dispose(); };
  }, [dispose]);
  useEffect(() => {
    if (owner.current && owner.current.deviceId !== deviceId) {
      dispose();
      setResult(idle());
    }
  }, [deviceId, dispose]);
  const start = useCallback(async () => {
    if (owner.current && !owner.current.disposed && !owner.current.stopped) return;
    const request: Owner = { id: crypto.randomUUID(), deviceId, sequence: state?.sequence ?? -1, disposed: false, stopRequested: false, stopped: false, startSettled: false, noOwnerStopSequence: null };
    owner.current = request;
    setResult({ ...idle(), id: request.id, deviceId, state: 'starting', stopped: false });
    try {
      const next = await keyboard.action({ action: 'microphone_test_start', testId: request.id, audioDeviceId: deviceId });
      request.startSettled = true;
      if (!mounted.current || request.disposed || owner.current !== request) {
        await stopOwned(request.id);
        return;
      }
      observe(next);
      if (request.stopRequested && !request.stopped) observeStop(await stopOwned(request.id), request);
    } catch (failure) {
      request.startSettled = true;
      if (request.noOwnerStopSequence !== null && request.noOwnerStopSequence >= request.sequence) request.stopped = true;
      if (mounted.current && !request.disposed && owner.current === request) setResult(previous => ({ ...previous,
        state: 'failed', stopped: request.stopped, detail: failure instanceof Error ? failure.message : String(failure) }));
    }
  }, [deviceId, observe, observeStop, state?.sequence]);
  const stop = useCallback(async () => {
    const request = owner.current;
    if (!request || request.disposed) return;
    request.stopRequested = true;
    setResult(previous => ({ ...previous, state: 'stopping' }));
    try { observeStop(await stopOwned(request.id), request); }
    catch (failure) {
      if (mounted.current && owner.current === request && !request.disposed) setResult(previous => ({ ...previous,
        state: 'failed', detail: failure instanceof Error ? failure.message : String(failure) }));
    }
  }, [observeStop]);
  return { result, start, stop, active: Boolean(result.id && !result.stopped) };
}
