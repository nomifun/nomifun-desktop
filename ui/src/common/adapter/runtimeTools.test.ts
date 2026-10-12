import { afterEach, expect, spyOn, test } from 'bun:test';
import { ensureManagedBun, runtimeTools, type RuntimeToolSnapshot } from './runtimeTools';
import { application } from './ipcBridge';

const restores: Array<() => void> = [];
const spy = <T extends { mockRestore(): void }>(value: T): T => {
  restores.push(() => value.mockRestore()); return value;
};
afterEach(() => { while (restores.length) restores.pop()!(); });
const snapshot = (phase: RuntimeToolSnapshot['phase'], revision = 1): RuntimeToolSnapshot => ({
  tool: 'bun', version: '1.3.13', revision, phase, received_bytes: 0,
});

test('user paths and ordinary commands never start a component download', async () => {
  const begin = spy(spyOn(runtimeTools.prepare, 'invoke'));
  for (const command of ['/user/tools/bun', 'node', 'bash', 'bun test']) {
    expect(await ensureManagedBun(command, { isCurrent: () => true })).toBe(true);
  }
  expect(begin).not.toHaveBeenCalled();
});

test('preparation reports real download state and settles only after ready', async () => {
  spy(spyOn(runtimeTools.prepare, 'invoke').mockResolvedValue({ ...snapshot('downloading'), received_bytes: 100, total_bytes: 200 }));
  const status = spy(spyOn(runtimeTools.status, 'invoke').mockResolvedValue(snapshot('ready', 2)));
  const progress: RuntimeToolSnapshot[] = [];
  let settled = false;
  const pending = ensureManagedBun('bunx', { isCurrent: () => true, onProgress: (value) => progress.push(value) }).then((ready) => { settled = true; return ready; });
  await Promise.resolve();
  expect(settled).toBe(false);
  expect(await pending).toBe(true);
  expect(progress[0]?.received_bytes).toBe(100);
  expect(status).toHaveBeenCalledTimes(1);
});

test('Windows aliases follow the backend host rather than the browser operating system', async () => {
  const info = spy(spyOn(application.systemInfo, 'invoke').mockResolvedValue({ platform: 'linux', arch: 'x86_64', cacheDir: '', dataDir: '', workDir: '', logDir: '', storageGeneration: 'v3', agentDataGeneration: 1 }));
  const begin = spy(spyOn(runtimeTools.prepare, 'invoke').mockResolvedValue(snapshot('ready')));
  expect(await ensureManagedBun('BUN.exe', { isCurrent: () => true })).toBe(true);
  expect(begin).not.toHaveBeenCalled();
  info.mockResolvedValue({ platform: 'win32', arch: 'x64', cacheDir: '', dataDir: '', workDir: '', logDir: '', storageGeneration: 'v3', agentDataGeneration: 1 });
  expect(await ensureManagedBun('BUN.exe', { isCurrent: () => true })).toBe(true);
  expect(begin).toHaveBeenCalledTimes(1);
});

test('a detached launch cannot start its user operation after shared preparation completes', async () => {
  let finish!: (value: RuntimeToolSnapshot) => void;
  spy(spyOn(runtimeTools.prepare, 'invoke').mockImplementation(() => new Promise((resolve) => { finish = resolve; })));
  let current = true;
  const pending = ensureManagedBun('bun', { isCurrent: () => current });
  current = false;
  finish(snapshot('ready'));
  expect(await pending).toBe(false);
});

test('wrong component identity and explicit supplier failures stop the dependent operation', async () => {
  const begin = spy(spyOn(runtimeTools.prepare, 'invoke').mockResolvedValue({ ...snapshot('ready'), tool: 'node' }));
  await expect(ensureManagedBun('bun', { isCurrent: () => true })).rejects.toThrow('Invalid');
  begin.mockResolvedValue({ ...snapshot('error'), error: { code: 'DOWNLOAD_FAILED', message: 'Component download failed', retryable: true } });
  await expect(ensureManagedBun('bun', { isCurrent: () => true })).rejects.toThrow('Component download failed');
});
