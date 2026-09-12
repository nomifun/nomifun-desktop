import { afterEach, beforeEach, describe, expect, test } from 'bun:test';
import { act, cleanup, renderHook } from '@testing-library/react';
import { ConfigServiceImpl } from '@/common/config/configService';
import useTheme, { type Theme } from './useTheme';

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}

function fixture() {
  const gets: ReturnType<typeof deferred<unknown>>[] = [];
  const puts: Array<{ body: unknown; reply: ReturnType<typeof deferred<unknown>> }> = [];
  const config = new ConfigServiceImpl(<T>(method: string, _path: string, body?: unknown) => {
    const reply = deferred<unknown>();
    if (method === 'GET') gets.push(reply);
    else puts.push({ body, reply });
    return reply.promise as Promise<T>;
  });
  const broadcasts: Theme[] = [];
  const broadcast = (theme: Theme) => { broadcasts.push(theme); };
  const mount = () => renderHook(() => useTheme(config, broadcast));
  return { config, gets, puts, broadcasts, mount };
}

let hint: string | null;
let htmlTheme: string | null;
let bodyTheme: string | null;
beforeEach(() => {
  hint = localStorage.getItem('__nomifun_theme');
  htmlTheme = document.documentElement.getAttribute('data-theme');
  bodyTheme = document.body.getAttribute('arco-theme');
  localStorage.removeItem('__nomifun_theme');
});
afterEach(() => {
  cleanup();
  if (hint === null) localStorage.removeItem('__nomifun_theme');
  else localStorage.setItem('__nomifun_theme', hint);
  if (htmlTheme === null) document.documentElement.removeAttribute('data-theme');
  else document.documentElement.setAttribute('data-theme', htmlTheme);
  if (bodyTheme === null) document.body.removeAttribute('arco-theme');
  else document.body.setAttribute('arco-theme', bodyTheme);
});

describe('useTheme with the real config service', () => {
  test('absent theme after offline initialization replaces the dark hint with the default', async () => {
    localStorage.setItem('__nomifun_theme', 'dark');
    const f = fixture();
    const hook = f.mount();
    await act(async () => { await Promise.resolve(); f.gets[0]!.reject(new Error('test offline')); });
    expect(hook.result.current[0]).toBe('dark');
    expect(f.config.get('theme')).toBeUndefined();
    let reload!: Promise<void>;
    await act(async () => { reload = f.config.reload(); });
    await act(async () => { f.gets[1]!.resolve({}); await reload; });
    expect(f.config.isInitialized()).toBe(true);
    expect(f.config.get('theme')).toBeUndefined();
    expect(hook.result.current[0]).toBe('light');
    expect(document.documentElement.getAttribute('data-theme')).toBe('light');
    expect(document.body.getAttribute('arco-theme')).toBe('light');
    expect(localStorage.getItem('__nomifun_theme')).toBe('light');
  });

  test('keeps the pre-login hint, then applies reload and deletion to state, DOM and hint', async () => {
    localStorage.setItem('__nomifun_theme', 'dark');
    const f = fixture();
    const hook = f.mount();
    await act(async () => { await Promise.resolve(); f.gets[0]!.reject(new Error('test offline')); });
    expect(hook.result.current[0]).toBe('dark');
    let reload!: Promise<void>;
    await act(async () => { reload = f.config.reload(); });
    await act(async () => { f.gets[1]!.resolve({ theme: 'light' }); await reload; });
    expect(hook.result.current[0]).toBe('light');
    expect(document.documentElement.getAttribute('data-theme')).toBe('light');
    expect(document.body.getAttribute('arco-theme')).toBe('light');
    expect(localStorage.getItem('__nomifun_theme')).toBe('light');
    await act(async () => { reload = f.config.reload(); });
    await act(async () => { f.gets[2]!.resolve({ theme: 'dark' }); await reload; });
    await act(async () => { reload = f.config.reload(); });
    await act(async () => { f.gets[3]!.resolve({}); await reload; });
    expect(hook.result.current[0]).toBe('light');
    expect(document.body.getAttribute('arco-theme')).toBe('light');
    expect(f.puts).toHaveLength(0);
    expect(f.broadcasts).toEqual([]);
  });

  test('a rejected write reconciles cache, DOM and hint with persisted state', async () => {
    const f = fixture();
    const hook = f.mount();
    await act(async () => { await Promise.resolve(); f.gets[0]!.resolve({ theme: 'light' }); });
    let writing!: Promise<void>;
    act(() => { writing = hook.result.current[1]('dark'); });
    expect(f.config.get('theme')).toBe('dark');
    await act(async () => { f.puts[0]!.reply.reject(new Error('test write rejected')); });
    expect(f.gets).toHaveLength(2);
    await act(async () => { f.gets[1]!.resolve({ theme: 'light' }); await writing; });
    expect(f.config.get('theme')).toBe('light');
    expect(hook.result.current[0]).toBe('light');
    expect(document.body.getAttribute('arco-theme')).toBe('light');
    expect(localStorage.getItem('__nomifun_theme')).toBe('light');
    expect(f.broadcasts).toEqual([]);
  });

  test('an older failure cannot roll back a newer pending choice', async () => {
    const f = fixture();
    const hook = f.mount();
    await act(async () => { await Promise.resolve(); f.gets[0]!.resolve({ theme: 'dark' }); });
    let first!: Promise<void>;
    let second!: Promise<void>;
    act(() => { first = hook.result.current[1]('light'); });
    act(() => { second = hook.result.current[1]('dark'); });
    await act(async () => { f.puts[0]!.reply.reject(new Error('test older failure')); });
    expect(f.gets).toHaveLength(2);
    await act(async () => { f.gets[1]!.resolve({ theme: 'dark' }); await first; });
    expect(f.config.get('theme')).toBe('dark');
    expect(hook.result.current[0]).toBe('dark');
    await act(async () => { f.puts[1]!.reply.resolve(undefined); await second; });
    expect(f.broadcasts).toEqual(['dark']);
  });

  test('a late success does not broadcast an obsolete choice', async () => {
    const f = fixture();
    const hook = f.mount();
    await act(async () => { await Promise.resolve(); f.gets[0]!.resolve({ theme: 'light' }); });
    let first!: Promise<void>;
    let second!: Promise<void>;
    act(() => { first = hook.result.current[1]('dark'); });
    act(() => { second = hook.result.current[1]('light'); });
    await act(async () => { f.puts[1]!.reply.resolve(undefined); await second; });
    await act(async () => { f.puts[0]!.reply.resolve(undefined); await first; });
    expect(f.broadcasts).toEqual(['light']);
    expect(hook.result.current[0]).toBe('light');
  });

  test('unmount prevents an outstanding initialization from changing the DOM', async () => {
    const f = fixture();
    const hook = f.mount();
    await act(async () => {});
    hook.unmount();
    document.documentElement.setAttribute('data-theme', 'after-unmount');
    document.body.setAttribute('arco-theme', 'after-unmount');
    await act(async () => { f.gets[0]!.resolve({ theme: 'dark' }); });
    expect(document.documentElement.getAttribute('data-theme')).toBe('after-unmount');
    expect(document.body.getAttribute('arco-theme')).toBe('after-unmount');
  });

  test('invalid persisted themes use the supported default', async () => {
    const f = fixture();
    const hook = f.mount();
    await act(async () => { await Promise.resolve(); f.gets[0]!.resolve({ theme: 'legacy-theme' }); });
    expect(hook.result.current[0]).toBe('light');
    expect(document.body.getAttribute('arco-theme')).toBe('light');
  });

  test('two rejected writes reconcile even when the older failure arrives last', async () => {
    const f = fixture();
    const hook = f.mount();
    await act(async () => { await Promise.resolve(); f.gets[0]!.resolve({ theme: 'light' }); });
    let first!: Promise<void>;
    let second!: Promise<void>;
    act(() => { first = hook.result.current[1]('dark'); });
    act(() => { second = hook.result.current[1]('light'); });
    await act(async () => { f.puts[1]!.reply.reject(new Error('test newer failure')); });
    await act(async () => { f.gets[1]!.resolve({ theme: 'light' }); await second; });
    await act(async () => { f.puts[0]!.reply.reject(new Error('test older failure')); });
    await act(async () => { f.gets[2]!.resolve({ theme: 'light' }); await first; });
    expect(f.config.get('theme')).toBe('light');
    expect(hook.result.current[0]).toBe('light');
    expect(document.body.getAttribute('arco-theme')).toBe('light');
    expect(f.broadcasts).toEqual([]);
  });

  test('failed reconciliation retains the rollback hint without claiming initialized cache', async () => {
    const f = fixture();
    const hook = f.mount();
    await act(async () => { await Promise.resolve(); f.gets[0]!.resolve({ theme: 'dark' }); });
    let writing!: Promise<void>;
    act(() => { writing = hook.result.current[1]('light'); });
    await act(async () => { f.puts[0]!.reply.reject(new Error('test write rejected')); });
    await act(async () => { f.gets[1]!.reject(new Error('test offline')); await writing; });
    expect(f.config.isInitialized()).toBe(false);
    expect(f.config.get('theme')).toBeUndefined();
    expect(hook.result.current[0]).toBe('dark');
    expect(localStorage.getItem('__nomifun_theme')).toBe('dark');
  });

  test('unmount suppresses a late success broadcast', async () => {
    const f = fixture();
    const hook = f.mount();
    await act(async () => { await Promise.resolve(); f.gets[0]!.resolve({ theme: 'light' }); });
    let writing!: Promise<void>;
    act(() => { writing = hook.result.current[1]('dark'); });
    hook.unmount();
    await act(async () => { f.puts[0]!.reply.resolve(undefined); await writing; });
    expect(f.broadcasts).toEqual([]);
  });
});
