import { afterEach, expect, spyOn, test } from 'bun:test';
import { BackendHttpError } from './httpBridge';
import { downloadBunRuntime, isBunDownloadRequired, runtimeTools, type RuntimeToolSnapshot } from './runtimeTools';

const restores: Array<() => void> = [];
const spy = <T extends { mockRestore(): void }>(value: T): T => {
  restores.push(() => value.mockRestore()); return value;
};
afterEach(() => { while (restores.length) restores.pop()!(); });
const snapshot = (phase: RuntimeToolSnapshot['phase'], revision = 1): RuntimeToolSnapshot => ({
  tool: 'bun', version: '1.3.13', revision, phase, received_bytes: 0,
});
test('only the typed missing Bun component error can offer a runtime download', () => {
  const backendError = (code: string, details: unknown) => new BackendHttpError({
    method: 'POST', path: '/api/terminals', status: 409,
    body: { code, error: 'runtime error', details },
  });
  expect(isBunDownloadRequired(backendError('COMPONENT_PREPARATION_REQUIRED', { component: 'bun' }))).toBe(true);
  for (const error of [
    new Error('COMPONENT_PREPARATION_REQUIRED'),
    { code: 'COMPONENT_PREPARATION_REQUIRED', details: { component: 'bun' } },
    backendError('COMPONENT_UNAVAILABLE', { component: 'bun' }),
    backendError('MCP_COMMAND_START_FAILED', { component: 'bun' }),
    backendError('COMPONENT_PREPARATION_REQUIRED', { component: 'node' }),
    backendError('COMPONENT_PREPARATION_REQUIRED', { runtime: 'bun' }),
    backendError('COMPONENT_PREPARATION_REQUIRED', null),
    backendError('COMPONENT_PREPARATION_REQUIRED', [{ component: 'bun' }]),
  ]) expect(isBunDownloadRequired(error)).toBe(false);
});

test('an explicitly chosen download reports real progress and settles only after ready', async () => {
  const begin = spy(spyOn(runtimeTools.prepare, 'invoke').mockResolvedValue({ ...snapshot('downloading'), received_bytes: 100, total_bytes: 200 }));
  const status = spy(spyOn(runtimeTools.status, 'invoke').mockResolvedValue(snapshot('ready', 2)));
  const progress: RuntimeToolSnapshot[] = [];
  const pending = downloadBunRuntime({ isCurrent: () => true, onProgress: (value) => progress.push(value) });
  expect(await pending).toBe(true);
  expect(begin).toHaveBeenCalledTimes(1);
  expect(progress[0]?.received_bytes).toBe(100);
  expect(status).toHaveBeenCalledTimes(1);
});

test('a detached operation cannot start its chosen download', async () => {
  const begin = spy(spyOn(runtimeTools.prepare, 'invoke'));
  expect(await downloadBunRuntime({ isCurrent: () => false })).toBe(false);
  expect(begin).not.toHaveBeenCalled();
});

test('a detached launch cannot continue after its chosen download completes', async () => {
  let finish!: (value: RuntimeToolSnapshot) => void;
  const begin = spy(spyOn(runtimeTools.prepare, 'invoke').mockImplementation(() => new Promise((resolve) => { finish = resolve; })));
  let current = true;
  const pending = downloadBunRuntime({ isCurrent: () => current });
  expect(begin).toHaveBeenCalledTimes(1);
  current = false;
  finish(snapshot('ready'));
  expect(await pending).toBe(false);
});

test('wrong downloaded component identity and supplier failures stop the dependent operation', async () => {
  const begin = spy(spyOn(runtimeTools.prepare, 'invoke').mockResolvedValue({ ...snapshot('ready'), tool: 'node' }));
  await expect(downloadBunRuntime({ isCurrent: () => true })).rejects.toThrow('Invalid');
  begin.mockResolvedValue({ ...snapshot('error'), error: { code: 'DOWNLOAD_FAILED', message: 'Component download failed', retryable: true } });
  await expect(downloadBunRuntime({ isCurrent: () => true })).rejects.toThrow('Component download failed');
});
