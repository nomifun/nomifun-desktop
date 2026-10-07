import { afterEach, beforeEach, describe, expect, test } from 'bun:test';
import { act, cleanup, renderHook } from '@testing-library/react';
import { ConfigServiceImpl } from '@/common/config/configService';
import useColorScheme from './useColorScheme';

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}

function fixture() {
  const gets: ReturnType<typeof deferred<unknown>>[] = [];
  const puts: ReturnType<typeof deferred<unknown>>[] = [];
  const config = new ConfigServiceImpl(<T>(method: string) => {
    const reply = deferred<unknown>();
    (method === 'GET' ? gets : puts).push(reply);
    return reply.promise as Promise<T>;
  });
  const mount = () => renderHook(() => useColorScheme(config));
  return { config, gets, puts, mount };
}

let hint: string | null;
let attribute: string | null;
beforeEach(() => {
  hint = localStorage.getItem('__nomifun_colorScheme');
  attribute = document.documentElement.getAttribute('data-color-scheme');
  localStorage.removeItem('__nomifun_colorScheme');
});
afterEach(() => {
  cleanup();
  if (hint === null) localStorage.removeItem('__nomifun_colorScheme');
  else localStorage.setItem('__nomifun_colorScheme', hint);
  if (attribute === null) document.documentElement.removeAttribute('data-color-scheme');
  else document.documentElement.setAttribute('data-color-scheme', attribute);
});

describe('useColorScheme with the real config service', () => {
  test('a rejected write restores absence in the persisted cache', async () => {
    const f = fixture();
    const hook = f.mount();
    await act(async () => { await Promise.resolve(); f.gets[0]!.resolve({}); });
    let writing!: Promise<void>;
    act(() => { writing = hook.result.current[1]('default'); });
    await act(async () => { f.puts[0]!.reject(new Error('test write rejected')); });
    expect(f.gets).toHaveLength(2);
    await act(async () => { f.gets[1]!.resolve({}); await writing; });
    expect(f.config.get('colorScheme')).toBeUndefined();
    expect(hook.result.current[0]).toBe('default');
    expect(document.documentElement.getAttribute('data-color-scheme')).toBe('default');
    expect(localStorage.getItem('__nomifun_colorScheme')).toBe('default');
  });

  test('reload re-applies the only supported scheme without persisting legacy values', async () => {
    const f = fixture();
    const hook = f.mount();
    await act(async () => { await Promise.resolve(); f.gets[0]!.resolve({ colorScheme: 'default' }); });
    document.documentElement.setAttribute('data-color-scheme', 'legacy');
    let reload!: Promise<void>;
    await act(async () => { reload = f.config.reload(); });
    await act(async () => { f.gets[1]!.resolve({ colorScheme: 'removed-scheme' }); await reload; });
    expect(hook.result.current[0]).toBe('default');
    expect(document.documentElement.getAttribute('data-color-scheme')).toBe('default');
    expect(f.puts).toHaveLength(0);
  });

  test('late initialization cannot change the DOM after unmount', async () => {
    const f = fixture();
    const hook = f.mount();
    await act(async () => {});
    hook.unmount();
    document.documentElement.setAttribute('data-color-scheme', 'after-unmount');
    await act(async () => { f.gets[0]!.resolve({ colorScheme: 'default' }); });
    expect(document.documentElement.getAttribute('data-color-scheme')).toBe('after-unmount');
  });
});
