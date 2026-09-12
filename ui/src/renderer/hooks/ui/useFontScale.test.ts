import { afterEach, describe, expect, test } from 'bun:test';
import { act, cleanup, renderHook } from '@testing-library/react';
import { ConfigServiceImpl } from '@/common/config/configService';
import useFontScale, { clampFontScale } from './useFontScale';

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
  const native: Array<{ factor: number; reply: ReturnType<typeof deferred<number>> }> = [];
  let hold = false;
  let actual = 1;
  const setZoom = async ({ factor }: { factor: number }) => {
    const reply = deferred<number>();
    native.push({ factor, reply });
    if (!hold) reply.resolve(factor);
    actual = await reply.promise;
    return actual;
  };
  const mount = () => renderHook(() => useFontScale(config, setZoom));
  return { config, gets, puts, native, mount, hold: () => { hold = true; }, actual: () => actual };
}

afterEach(cleanup);

describe('useFontScale with the real config service', () => {
  test('initialization, reload and deletion apply native zoom without writing preferences', async () => {
    const f = fixture();
    const hook = f.mount();
    await act(async () => { await Promise.resolve(); f.gets[0]!.resolve({ 'ui.zoomFactor': 1.1 }); });
    expect(hook.result.current[0]).toBe(1.1);
    expect(f.actual()).toBe(1.1);
    let reload!: Promise<void>;
    await act(async () => { reload = f.config.reload(); });
    await act(async () => { f.gets[1]!.resolve({ 'ui.zoomFactor': 1.2 }); await reload; });
    expect(hook.result.current[0]).toBe(1.2);
    expect(f.actual()).toBe(1.2);
    await act(async () => { reload = f.config.reload(); });
    await act(async () => { f.gets[2]!.resolve({}); await reload; });
    expect(hook.result.current[0]).toBe(1);
    expect(f.actual()).toBe(1);
    expect(f.puts).toHaveLength(0);
  });

  test('failed persistence rolls back cache, slider and native zoom', async () => {
    const f = fixture();
    const hook = f.mount();
    await act(async () => { await Promise.resolve(); f.gets[0]!.resolve({ 'ui.zoomFactor': 1 }); });
    let writing!: Promise<void>;
    await act(async () => { writing = hook.result.current[1](1.2); });
    expect(f.config.get('ui.zoomFactor')).toBe(1.2);
    await act(async () => { f.puts[0]!.reply.reject(new Error('test write rejected')); });
    expect(f.gets).toHaveLength(2);
    await act(async () => { f.gets[1]!.resolve({ 'ui.zoomFactor': 1 }); await writing; });
    expect(f.config.get('ui.zoomFactor')).toBe(1);
    expect(hook.result.current[0]).toBe(1);
    expect(f.actual()).toBe(1);
  });

  test('native failure does not persist the rejected scale and restores the cached scale', async () => {
    const f = fixture();
    const hook = f.mount();
    await act(async () => { await Promise.resolve(); f.gets[0]!.resolve({ 'ui.zoomFactor': 1 }); });
    f.hold();
    let writing!: Promise<void>;
    await act(async () => { writing = hook.result.current[1](1.2); });
    await act(async () => { f.native.at(-1)!.reply.reject(new Error('test native failure')); await writing; });
    expect(f.puts).toHaveLength(0);
    expect(f.config.get('ui.zoomFactor')).toBe(1);
    expect(hook.result.current[0]).toBe(1);
    expect(f.native.at(-1)!.factor).toBe(1);
    await act(async () => { f.native.at(-1)!.reply.resolve(1); });
  });

  test('serializes native calls and never persists a superseded slider action', async () => {
    const f = fixture();
    const hook = f.mount();
    await act(async () => { await Promise.resolve(); f.gets[0]!.resolve({ 'ui.zoomFactor': 1 }); });
    f.hold();
    const before = f.native.length;
    let first!: Promise<void>;
    let second!: Promise<void>;
    await act(async () => { first = hook.result.current[1](1.1); });
    await act(async () => { second = hook.result.current[1](1.2); });
    expect(f.native).toHaveLength(before + 1);
    await act(async () => { f.native[before]!.reply.resolve(1.1); await first; });
    expect(f.puts).toHaveLength(0);
    expect(f.native.at(-1)!.factor).toBe(1.2);
    await act(async () => { f.native.at(-1)!.reply.resolve(1.2); });
    expect(f.puts.map((put) => put.body)).toEqual([{ 'ui.zoomFactor': 1.2 }]);
    await act(async () => { f.puts[0]!.reply.resolve(undefined); await second; });
    expect(f.config.get('ui.zoomFactor')).toBe(1.2);
    expect(hook.result.current[0]).toBe(1.2);
    expect(f.actual()).toBe(1.2);
  });

  test('an in-flight startup zoom finishes before a newer user zoom is applied', async () => {
    const f = fixture();
    f.hold();
    const hook = f.mount();
    await act(async () => { await Promise.resolve(); f.gets[0]!.resolve({ 'ui.zoomFactor': 1.1 }); });
    const firstNative = f.native[0]!;
    let writing!: Promise<void>;
    await act(async () => { writing = hook.result.current[1](1.2); });
    expect(f.native).toHaveLength(1);
    await act(async () => { firstNative.reply.resolve(firstNative.factor); });
    expect(f.native.at(-1)!.factor).toBe(1.2);
    await act(async () => { f.native.at(-1)!.reply.resolve(1.2); });
    await act(async () => { f.puts[0]!.reply.resolve(undefined); await writing; });
    expect(f.actual()).toBe(1.2);
    expect(hook.result.current[0]).toBe(1.2);
  });

  test('unmount prevents a native completion from starting a PUT', async () => {
    const f = fixture();
    const hook = f.mount();
    await act(async () => { await Promise.resolve(); f.gets[0]!.resolve({ 'ui.zoomFactor': 1 }); });
    f.hold();
    let writing!: Promise<void>;
    await act(async () => { writing = hook.result.current[1](1.2); });
    hook.unmount();
    await act(async () => { f.native.at(-1)!.reply.resolve(1.2); });
    expect(f.puts).toHaveLength(0);
    await writing;
  });

  test('clamps invalid or unsupported numeric scales', () => {
    expect([NaN, Infinity, -Infinity, 0.2, 2, 1.15].map(clampFontScale)).toEqual([1, 1, 1, 0.8, 1.3, 1.15]);
  });

  test('a reload during native application does not replace the pending user choice', async () => {
    const f = fixture();
    const hook = f.mount();
    await act(async () => { await Promise.resolve(); f.gets[0]!.resolve({ 'ui.zoomFactor': 1 }); });
    f.hold();
    let writing!: Promise<void>;
    await act(async () => { writing = hook.result.current[1](1.2); });
    let reload!: Promise<void>;
    await act(async () => { reload = f.config.reload(); });
    await act(async () => { f.gets[1]!.resolve({ 'ui.zoomFactor': 1.3 }); await reload; });
    expect(hook.result.current[0]).toBe(1.2);
    expect(f.native.some((call) => call.factor === 1.3)).toBe(false);
    await act(async () => { f.native.at(-1)!.reply.resolve(1.2); });
    await act(async () => { f.puts[0]!.reply.resolve(undefined); await writing; });
    expect(f.config.get('ui.zoomFactor')).toBe(1.2);
    expect(f.actual()).toBe(1.2);
  });

  test('an older PUT failure cannot interrupt a newer native operation', async () => {
    const f = fixture();
    const hook = f.mount();
    await act(async () => { await Promise.resolve(); f.gets[0]!.resolve({ 'ui.zoomFactor': 1 }); });
    let first!: Promise<void>;
    let second!: Promise<void>;
    await act(async () => { first = hook.result.current[1](1.1); });
    f.hold();
    await act(async () => { second = hook.result.current[1](1.2); });
    await act(async () => { f.puts[0]!.reply.reject(new Error('test older failure')); });
    await act(async () => { f.gets[1]!.resolve({ 'ui.zoomFactor': 1 }); await first; });
    expect(hook.result.current[0]).toBe(1.2);
    expect(f.native.at(-1)!.factor).toBe(1.2);
    await act(async () => { f.native.at(-1)!.reply.resolve(1.2); });
    await act(async () => { f.puts[1]!.reply.resolve(undefined); await second; });
    expect(f.config.get('ui.zoomFactor')).toBe(1.2);
    expect(f.actual()).toBe(1.2);
  });

  test('persists the adapter result, including the WebUI no-op factor', async () => {
    const f = fixture();
    const hook = f.mount();
    await act(async () => { await Promise.resolve(); f.gets[0]!.resolve({ 'ui.zoomFactor': 1 }); });
    f.hold();
    let writing!: Promise<void>;
    await act(async () => { writing = hook.result.current[1](1.2); });
    await act(async () => { f.native.at(-1)!.reply.resolve(1); });
    expect(f.puts[0]!.body).toEqual({ 'ui.zoomFactor': 1 });
    await act(async () => { f.puts[0]!.reply.resolve(undefined); await writing; });
    expect(hook.result.current[0]).toBe(1);
    expect(f.config.get('ui.zoomFactor')).toBe(1);
  });
});
