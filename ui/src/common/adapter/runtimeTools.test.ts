import { afterEach, expect, spyOn, test } from 'bun:test';
import { withRuntimePreparation, runtimeTools, type RuntimeToolSnapshot } from './runtimeTools';

const restores: Array<() => void> = [];
const spy = <T extends { mockRestore(): void }>(value: T): T => {
  restores.push(() => value.mockRestore()); return value;
};
afterEach(() => { while (restores.length) restores.pop()!(); });
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
const pause = () => new Promise<void>(resolve => setTimeout(resolve, 15));

test('progress observation is read only and never replays the original launch', async () => {
  const prepare = spy(spyOn(runtimeTools.prepare, 'invoke'));
  spy(spyOn(runtimeTools.status, 'invoke').mockImplementation(async ({ tool }) => ({
    tool, version: 'fixture', revision: 1, phase: 'downloading', received_bytes: 100,
  })));
  const launched = deferred<string>();
  let launches = 0;
  const progress: RuntimeToolSnapshot[] = [];
  const operation = withRuntimePreparation(() => { launches += 1; return launched.promise; }, {
    isCurrent: () => true, onProgress: snapshot => progress.push(snapshot), pollIntervalMs: 1,
  });
  await pause();
  expect(progress.length).toBeGreaterThan(0);
  expect(prepare).not.toHaveBeenCalled();
  launched.resolve('actual-result');
  expect(await operation).toBe('actual-result');
  expect(launches).toBe(1);
});

test('leaving stops progress while preserving the real owner result', async () => {
  spy(spyOn(runtimeTools.status, 'invoke').mockResolvedValue({
    tool: 'bun', version: 'fixture', revision: 1, phase: 'downloading', received_bytes: 100,
  }));
  let current = true;
  const launched = deferred<string>();
  const progress: RuntimeToolSnapshot[] = [];
  const operation = withRuntimePreparation(() => launched.promise, {
    isCurrent: () => current, onProgress: snapshot => progress.push(snapshot), pollIntervalMs: 1,
  });
  current = false;
  await pause();
  expect(progress).toHaveLength(0);
  launched.resolve('completed');
  expect(await operation).toBe('completed');
});

test('status failure never hides the original launch error', async () => {
  spy(spyOn(runtimeTools.status, 'invoke').mockRejectedValue(new Error('status offline')));
  const launched = deferred<string>();
  const operation = withRuntimePreparation(() => launched.promise, {
    isCurrent: () => true, onProgress: () => {}, pollIntervalMs: 1,
  });
  await pause();
  launched.reject(new Error('actual spawn failure'));
  await expect(operation).rejects.toThrow('actual spawn failure');
});
