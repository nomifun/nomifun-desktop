/** Managed preparation precedes the deadline of the requested user operation. */
import { httpGet, httpPost } from './httpBridge';
import { application } from './ipcBridge';
import type { TFunction } from 'i18next';

export interface RuntimeToolSnapshot {
  tool: 'bun' | 'node' | 'officecli';
  version: string;
  revision: number;
  phase: 'idle' | 'checking' | 'downloading' | 'extracting' | 'verifying' | 'ready' | 'error';
  received_bytes: number;
  total_bytes?: number | null;
  error?: { code: string; message: string; retryable: boolean } | null;
}

export const runtimeTools = {
  status: httpGet<RuntimeToolSnapshot, { tool: RuntimeToolSnapshot['tool'] }>(
    ({ tool }) => `/api/runtime-tools/${tool}`, { timeoutMs: 5_000 }
  ),
  prepare: httpPost<RuntimeToolSnapshot, { tool: RuntimeToolSnapshot['tool'] }>(
    ({ tool }) => `/api/runtime-tools/${tool}/prepare`, () => undefined, { timeoutMs: 5_000 }
  ),
};

export function requiresManagedBun(command: string, windowsHost = false): boolean {
  // Explicit user paths and shell text retain their own executable semantics.
  return windowsHost ? /^(?:bun|bunx)(?:\.exe)?$/i.test(command) : command === 'bun' || command === 'bunx';
}

function validSnapshot(value: RuntimeToolSnapshot): boolean {
  return value?.tool === 'bun' && typeof value.version === 'string'
    && Number.isSafeInteger(value.revision) && value.revision >= 0
    && ['idle', 'checking', 'downloading', 'extracting', 'verifying', 'ready', 'error'].includes(value.phase)
    && Number.isSafeInteger(value.received_bytes) && value.received_bytes >= 0
    && (value.total_bytes == null || (Number.isSafeInteger(value.total_bytes) && value.total_bytes >= 0))
    && (value.phase !== 'error' || (typeof value.error?.message === 'string' && typeof value.error.retryable === 'boolean'));
}

/** Detaching stops this observer; the supplier retains other consumers. */
export async function ensureManagedBun(
  command: string,
  options: { isCurrent: () => boolean; onProgress?: (snapshot: RuntimeToolSnapshot) => void }
): Promise<boolean> {
  if (!requiresManagedBun(command, true)) return options.isCurrent();
  if (!options.isCurrent()) return false;
  if (!requiresManagedBun(command)) {
    // CLI aliases belong to the backend host, including remote WebUI hosts.
    const info = await application.systemInfo.invoke();
    if (!options.isCurrent()) return false;
    if (info.platform !== 'win32') return true;
  }
  let snapshot = await runtimeTools.prepare.invoke({ tool: 'bun' });
  let revision = -1;
  const deadline = performance.now() + 11 * 60_000;
  while (options.isCurrent()) {
    if (!validSnapshot(snapshot) || snapshot.revision < revision) throw new Error('Invalid runtime preparation response');
    revision = snapshot.revision;
    if (snapshot.phase === 'ready') return true;
    if (snapshot.phase === 'error') throw new Error(snapshot.error!.message);
    options.onProgress?.(snapshot);
    if (performance.now() >= deadline) throw new Error('Runtime preparation timed out; retry preparation');
    await new Promise<void>((resolve) => setTimeout(resolve, 500));
    if (!options.isCurrent()) return false;
    snapshot = await runtimeTools.status.invoke({ tool: 'bun' });
  }
  return false;
}

export function formatRuntimePreparation(snapshot: RuntimeToolSnapshot, t: TFunction): string {
  const stages: Record<RuntimeToolSnapshot['phase'], string> = {
    idle: 'common.runtimePreparation.checking', checking: 'common.runtimePreparation.checking',
    downloading: 'common.runtimePreparation.downloading', extracting: 'common.runtimePreparation.extracting',
    verifying: 'common.runtimePreparation.verifying', ready: 'common.runtimePreparation.ready',
    error: 'common.runtimePreparation.failed',
  };
  const stage = t(stages[snapshot.phase], { tool: 'Bun' });
  if (snapshot.phase !== 'downloading') return stage;
  const received = (snapshot.received_bytes / 1_000_000).toFixed(1);
  return snapshot.total_bytes ? `${stage} ${received} / ${(snapshot.total_bytes / 1_000_000).toFixed(1)} MB` : `${stage} ${received} MB`;
}
