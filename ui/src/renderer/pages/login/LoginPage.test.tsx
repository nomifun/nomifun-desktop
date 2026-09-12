import { afterEach, beforeAll, describe, expect, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { MemoryRouter, Navigate, Route, Routes, useLocation, useNavigate } from 'react-router-dom';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { AuthProvider, useAuth } from '../../hooks/context/AuthContext';
import { configService } from '@/common/config/configService';
import loginStrings from '../../services/i18n/locales/en-US/login.json';

let LoginPage: typeof import('./index').default;
const locale = createInstance();
const restore: Array<() => void> = [];
const preferenceKeys = ['rememberMe', 'rememberedUsername', 'rememberedPassword'];
const user = { user_id: '0190f5fe-7c00-7a00-8000-000000000064', username: 'fixture-user' };
const response = (status: number, body: unknown) => new Response(JSON.stringify(body), {
  status, headers: { 'Content-Type': 'application/json' },
});

beforeAll(async () => {
  LoginPage = (await import('./index')).default;
  await locale.init({ lng: 'en-US', resources: { 'en-US': { translation: { login: loginStrings } } } });
});

afterEach(() => {
  cleanup();
  restore.splice(0).reverse().forEach((dispose) => dispose());
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

function fixture(needsSetup = false) {
  const previousStorage = Object.getOwnPropertyDescriptor(globalThis, 'localStorage');
  const store = new Map<string, string>();
  const storage = {
    getItem: (key: string) => store.get(key) ?? null,
    setItem: (key: string, value: string) => { store.set(key, value); },
    removeItem: (key: string) => { store.delete(key); },
    key: (index: number) => [...store.keys()][index] ?? null,
    get length() { return store.size; },
  };
  Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: storage });
  const title = document.title;
  const lang = document.documentElement.lang;
  restore.push(() => {
    if (previousStorage) Object.defineProperty(globalThis, 'localStorage', previousStorage);
    else delete (globalThis as { localStorage?: Storage }).localStorage;
    document.title = title;
    document.documentElement.lang = lang;
  });

  const requests: Array<{ path: string; options?: RequestInit; reply: ReturnType<typeof deferred<Response>> }> = [];
  const fetching = spyOn(globalThis, 'fetch').mockImplementation((input, options) => {
    const path = String(input);
    if (path === '/api/auth/status') return Promise.resolve(response(200, { success: true, needs_setup: needsSetup }));
    if (path === '/api/auth/user') return Promise.resolve(response(401, { success: false }));
    const reply = deferred<Response>();
    requests.push({ path, options, reply });
    return reply.promise;
  });
  const reloading = spyOn(configService, 'reload').mockResolvedValue(undefined);
  restore.push(() => fetching.mockRestore(), () => reloading.mockRestore());

  // Track real timers, including old broken callbacks, so a red run cannot
  // leak navigation or message callbacks into the next test.
  const ids: number[] = [];
  const originalSetTimeout = window.setTimeout;
  const setTimeout = originalSetTimeout.bind(window);
  const trackTimeout = Object.assign((callback: TimerHandler, delay?: number, ...args: unknown[]) => {
    const id = setTimeout(callback, delay, ...args);
    ids.push(id);
    return id;
  }, originalSetTimeout);
  const timing = spyOn(window, 'setTimeout').mockImplementation(trackTimeout);
  restore.push(() => { ids.forEach((id) => window.clearTimeout(id)); timing.mockRestore(); });

  let auth!: ReturnType<typeof useAuth>;
  let navigate!: ReturnType<typeof useNavigate>;
  const Probe = () => {
    auth = useAuth();
    navigate = useNavigate();
    return <output data-testid='path'>{useLocation().pathname}</output>;
  };
  const LoginRoute = () => useAuth().status === 'authenticated' ? <Navigate to='/guid' replace /> : <LoginPage />;
  const mount = async () => {
    let view!: ReturnType<typeof render>;
    await act(async () => {
      view = render(
        <I18nextProvider i18n={locale}>
          <AuthProvider>
            <MemoryRouter initialEntries={['/login']}>
              <Probe />
              <Routes>
                <Route path='/login' element={<LoginRoute />} />
                <Route path='*' element={<div>Other page</div>} />
              </Routes>
            </MemoryRouter>
          </AuthProvider>
        </I18nextProvider>
      );
    });
    const fill = () => {
      fireEvent.change(view.getByLabelText('Username'), { target: { value: '  fixture-user  ' } });
      fireEvent.change(view.getByLabelText('Password'), { target: { value: 'fixture-password' } });
    };
    const submit = () => fireEvent.submit(view.container.querySelector('form')!);
    const finish = async (status: number, body: unknown, index = requests.length - 1) => {
      await act(async () => { requests[index]!.reply.resolve(response(status, body)); });
    };
    return { ...view, fill, submit, finish };
  };
  return { requests, mount, timing, auth: () => auth, go: (path: string) => navigate(path) };
}

describe('LoginPage with real AuthProvider and route transitions', () => {
  test('empty fields do not dispatch; login trims username but not password', async () => {
    const f = fixture();
    const v = await f.mount();
    v.submit();
    expect(f.requests).toHaveLength(0);
    expect(v.getByRole('alert').textContent).toBe(loginStrings.errors.empty);
    v.fill();
    fireEvent.change(v.getByLabelText('Password'), { target: { value: ' password with spaces ' } });
    v.submit();
    expect(f.requests[0]!.path).toBe('/login');
    expect(f.requests[0]!.options?.credentials).toBe('include');
    expect(JSON.parse(f.requests[0]!.options?.body as string)).toEqual({ username: 'fixture-user', password: ' password with spaces ' });
    await v.finish(401, { success: false });
  });

  test.each([false, true])('coalesces repeated form submits while setup=%s is pending and permits retry', async (setup) => {
    const f = fixture(setup);
    const v = await f.mount();
    v.fill();
    act(() => { v.submit(); v.submit(); });
    const count = f.requests.length;
    for (let i = 0; i < count; i++) await v.finish(429, { success: false }, i);
    expect(count).toBe(1);
    expect(f.requests[0]!.path).toBe(setup ? '/api/auth/setup' : '/login');
    expect(v.getByRole('alert').textContent).toBe(loginStrings.errors.tooManyAttempts);
    v.submit();
    expect(f.requests).toHaveLength(2);
    await v.finish(500, { success: false });
    expect((v.getByRole('button', { name: setup ? loginStrings.setupSubmit : loginStrings.submit }) as HTMLButtonElement).disabled).toBe(false);
  });

  test('setup conflict shows backend message then switches to login', async () => {
    const f = fixture(true);
    const v = await f.mount();
    expect(v.queryByLabelText('Remember me')).toBe(null);
    expect(v.getByLabelText('Password').getAttribute('autocomplete')).toBe('new-password');
    v.fill(); v.submit();
    await v.finish(409, { success: false, message: 'Already initialized' });
    expect(v.getByRole('alert').textContent).toBe('Already initialized');
    expect(v.getByLabelText('Password').getAttribute('autocomplete')).toBe('current-password');
    v.submit();
    expect(f.requests[1]!.path).toBe('/login');
    await v.finish(401, { success: false });
  });

  test('successful login leaves navigation to auth routing, with no late redirect after moving on', async () => {
    const f = fixture();
    const v = await f.mount();
    v.fill(); v.submit();
    await v.finish(200, { success: true, user });
    expect(v.getByTestId('path').textContent).toBe('/guid');
    expect(document.body.classList.contains('login-page-active')).toBe(false);
    await act(async () => { f.go('/settings/system'); });
    await act(async () => { await new Promise((resolve) => setTimeout(resolve, 650)); });
    expect(v.getByTestId('path').textContent).toBe('/settings/system');
  });

  test('late success after leaving the page cannot write remembered credentials or schedule navigation', async () => {
    const f = fixture();
    const v = await f.mount();
    v.fill(); fireEvent.click(v.getByLabelText('Remember me')); v.submit();
    await act(async () => { f.go('/elsewhere'); });
    f.timing.mockClear();
    await v.finish(200, { success: true, user });
    expect(preferenceKeys.map((key) => localStorage.getItem(key))).toEqual([null, null, null]);
    expect(f.timing.mock.calls.filter((call) => call[1] === 600)).toHaveLength(0);
    expect(v.getByTestId('path').textContent).toBe('/elsewhere');
  });

  test('late error after unmount cannot create a message timer', async () => {
    const f = fixture();
    const v = await f.mount();
    v.fill(); v.submit();
    v.unmount();
    f.timing.mockClear();
    await v.finish(401, { success: false });
    expect(f.timing.mock.calls.filter((call) => call[1] === 5000)).toHaveLength(0);
  });

  test('unavailable local storage does not prevent rendering the login form', async () => {
    const f = fixture();
    const reading = spyOn(localStorage, 'getItem').mockImplementation(() => { throw new Error('fixture storage blocked'); });
    restore.push(() => reading.mockRestore());
    const v = await f.mount();
    expect(v.getByLabelText('Username')).toBeTruthy();
  });

  test('remembered fields round-trip only after success and logout returns to login', async () => {
    const f = fixture();
    const v = await f.mount();
    v.fill(); fireEvent.click(v.getByLabelText('Remember me')); v.submit();
    expect(localStorage.getItem('rememberMe')).toBe(null);
    await v.finish(200, { success: true, user });
    expect(localStorage.getItem('rememberMe')).toBe('true');
    let leaving!: Promise<void>;
    await act(async () => { leaving = f.auth().logout(); });
    expect(f.requests[1]!.path).toBe('/logout');
    await v.finish(200, { success: true });
    await act(async () => { await leaving; f.go('/login'); });
    expect((v.getByLabelText('Username') as HTMLInputElement).value).toBe('fixture-user');
    expect((v.getByLabelText('Password') as HTMLInputElement).value).toBe('fixture-password');
    fireEvent.click(v.getByLabelText('Remember me')); v.submit();
    await v.finish(200, { success: true, user });
    expect(preferenceKeys.map((key) => localStorage.getItem(key))).toEqual([null, null, null]);
  });

  test.each([true, false])('storage mutation failure cannot reject successful login (remember=%s)', async (remember) => {
    const f = fixture();
    const v = await f.mount();
    v.fill();
    if (remember) fireEvent.click(v.getByLabelText('Remember me'));
    const writing = spyOn(localStorage, remember ? 'setItem' : 'removeItem').mockImplementation(() => {
      throw new Error('fixture storage mutation blocked');
    });
    const warning = spyOn(console, 'warn').mockImplementation(() => {});
    restore.push(() => writing.mockRestore(), () => warning.mockRestore());
    v.submit();
    await v.finish(200, { success: true, user });
    expect(writing).toHaveBeenCalled();
    expect(warning).toHaveBeenCalledWith('Unable to save remembered login preferences');
    expect(v.getByTestId('path').textContent).toBe('/guid');
  });

  test('successful setup redirects without remembering the initial admin password', async () => {
    const f = fixture(true);
    const v = await f.mount();
    v.fill(); v.submit();
    await v.finish(200, { success: true, user });
    expect(v.getByTestId('path').textContent).toBe('/guid');
    expect(preferenceKeys.map((key) => localStorage.getItem(key))).toEqual([null, null, null]);
  });

  test('corrupt remembered fields stay empty; focus and password visibility work after the auth probe', async () => {
    const f = fixture();
    localStorage.setItem('rememberMe', 'true');
    localStorage.setItem('rememberedUsername', '%%%');
    localStorage.setItem('rememberedPassword', '%%%');
    const v = await f.mount();
    expect((v.getByLabelText('Username') as HTMLInputElement).value).toBe('');
    expect((v.getByLabelText('Password') as HTMLInputElement).value).toBe('');
    expect(document.activeElement).toBe(v.getByLabelText('Username'));
    expect(v.getByLabelText('Change language')).toBeTruthy();
    fireEvent.click(v.getByRole('button', { name: 'Show password' }));
    expect(v.getByLabelText('Password').getAttribute('type')).toBe('text');
    fireEvent.click(v.getByRole('button', { name: 'Hide password' }));
    expect(v.getByLabelText('Password').getAttribute('type')).toBe('password');
  });

  test.each([
    [401, loginStrings.errors.invalidCredentials],
    [500, loginStrings.errors.serverError],
    [403, 'Security token expired. Please try again.'],
    [400, '<script>fixture error</script>'],
  ] as const)('HTTP %s displays a safe error and its message timer expires', async (status, message) => {
    const f = fixture();
    const v = await f.mount();
    v.fill(); v.submit();
    await v.finish(status, { success: false, message: '<script>fixture error</script>' });
    expect(v.getByRole('alert').textContent).toBe(message);
    expect(v.container.querySelector('script')).toBe(null);
    const callback = f.timing.mock.calls.findLast((call) => call[1] === 5000)![0];
    act(() => { if (typeof callback === 'function') callback(); });
    expect(v.queryByRole('alert')).toBe(null);
  });

  test.each([false, true])('transport failure releases submission for retry (unexpected=%s)', async (unexpected) => {
    const f = fixture();
    const v = await f.mount();
    const logging = spyOn(console, 'error').mockImplementation(() => {});
    restore.push(() => logging.mockRestore());
    v.fill(); v.submit();
    // AuthProvider maps Error to networkError; a null rejection currently
    // escapes its own catch. The page must recover from either contract path.
    await act(async () => { f.requests[0]!.reply.reject(unexpected ? null : new Error('fixture offline')); });
    expect(v.getByRole('alert').textContent).toBe(unexpected ? loginStrings.errors.unknown : loginStrings.errors.networkError);
    v.submit();
    expect(f.requests).toHaveLength(2);
    await v.finish(401, { success: false });
  });

  test('edits during an in-flight login do not replace the submitted preference snapshot', async () => {
    const f = fixture();
    const v = await f.mount();
    v.fill(); fireEvent.click(v.getByLabelText('Remember me')); v.submit();
    fireEvent.change(v.getByLabelText('Username'), { target: { value: 'not-submitted' } });
    fireEvent.change(v.getByLabelText('Password'), { target: { value: 'not-submitted' } });
    await v.finish(200, { success: true, user });
    const decode = (key: string) => decodeURIComponent(atob(localStorage.getItem(key)!.split('').reverse().join('')));
    expect(decode('rememberedUsername')).toBe('fixture-user');
    expect(decode('rememberedPassword')).toBe('fixture-password');
  });
});
