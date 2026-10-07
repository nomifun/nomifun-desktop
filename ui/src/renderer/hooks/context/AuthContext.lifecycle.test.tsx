import { afterEach, expect, spyOn, test } from 'bun:test';
import { act, cleanup, render } from '@testing-library/react';
import { AuthProvider, useAuth } from './AuthContext';
import { AUTH_EXPIRED_EVENT } from '@/common/adapter/httpBridge';

let restoreFetch: (() => void) | undefined;
afterEach(() => { cleanup(); restoreFetch?.(); });

function fixture() {
  const pending: Array<{ path: string; signal?: AbortSignal | null; reply: (body: unknown) => void }> = [];
  const fetching = spyOn(globalThis, 'fetch').mockImplementation((input, init) => new Promise<Response>((resolve) => {
    pending.push({
      path: String(input), signal: init?.signal,
      reply: (body) => resolve(new Response(JSON.stringify(body), { headers: { 'Content-Type': 'application/json' } })),
    });
  }));
  restoreFetch = () => fetching.mockRestore();
  let auth!: ReturnType<typeof useAuth>;
  const Probe = () => { auth = useAuth(); return null; };
  const mount = () => render(<AuthProvider><Probe /></AuthProvider>);
  const answer = async (index: number, body: unknown) => {
    await act(async () => { pending[index]!.reply(body); });
  };
  return { pending, mount, answer, auth: () => auth };
}

const signedIn = { success: true, user: { user_id: '0190f5fe-7c00-7a00-8000-000000000071', username: 'fixture-user' } };

test.each(['status', 'user'])('expired auth cannot be overwritten by a cancelled %s probe', async (phase) => {
  const f = fixture();
  f.mount();
  if (phase === 'user') await f.answer(0, { success: true, needs_setup: false });
  const current = f.pending.length - 1;
  act(() => window.dispatchEvent(new Event(AUTH_EXPIRED_EVENT)));
  expect(f.pending[current]!.signal?.aborted).toBe(true);
  await f.answer(current, phase === 'status' ? { success: true, needs_setup: true } : signedIn);
  expect(f.auth().status).toBe('unauthenticated');
  expect(f.auth().needsSetup).toBe(false);
  expect(f.auth().user).toBe(null);
  expect(f.pending).toHaveLength(current + 1);
});

test('a superseded refresh cannot start another user probe or overwrite setup state', async () => {
  const f = fixture();
  f.mount();
  let refreshed!: Promise<void>;
  act(() => { refreshed = f.auth().refresh(); });
  expect(f.pending[0]!.signal?.aborted).toBe(true);
  await f.answer(1, { success: true, needs_setup: false });
  await f.answer(2, signedIn);
  await refreshed;
  await f.answer(0, { success: true, needs_setup: true });
  expect(f.auth().status).toBe('authenticated');
  expect(f.auth().needsSetup).toBe(false);
  expect(f.auth().user?.username).toBe('fixture-user');
  expect(f.pending).toHaveLength(3);
});

test('unmount aborts startup without dispatching the next probe', async () => {
  const f = fixture();
  const view = f.mount();
  view.unmount();
  expect(f.pending[0]!.signal?.aborted).toBe(true);
  await f.answer(0, { success: true, needs_setup: false });
  expect(f.pending).toHaveLength(1);
});
