/** Read-only progress for runtime preparation owned by the original launch. */
import { httpGet, httpPost } from './httpBridge';
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

function validSnapshot(value: RuntimeToolSnapshot, tool: RuntimeToolSnapshot['tool']): boolean {
  return value?.tool === tool && typeof value.version === 'string'
    && Number.isSafeInteger(value.revision) && value.revision >= 0
    && ['idle', 'checking', 'downloading', 'extracting', 'verifying', 'ready', 'error'].includes(value.phase)
    && Number.isSafeInteger(value.received_bytes) && value.received_bytes >= 0;
}

/** Observe the owner's automatic preparation without starting another download
 * or replaying a launch that may already have produced effects. Status failures
 * never replace the actual launch result. Leaving the view stops observation. */
export async function withRuntimePreparation<T>(operation: () => Promise<T>, options: {
  isCurrent: () => boolean;
  onProgress: (snapshot: RuntimeToolSnapshot) => void;
  pollIntervalMs?: number;
}): Promise<T> {
  let settled = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const revisions = new Map<RuntimeToolSnapshot['tool'], number>();
  const poll = async () => {
    if (settled || !options.isCurrent()) return;
    await Promise.allSettled((['bun', 'node', 'officecli'] as const).map(async tool => {
      const snapshot = await runtimeTools.status.invoke({ tool });
      if (settled || !options.isCurrent() || !validSnapshot(snapshot, tool)) return;
      if (snapshot.revision < (revisions.get(tool) ?? -1)) return;
      revisions.set(tool, snapshot.revision);
      if (!['idle', 'ready'].includes(snapshot.phase)) options.onProgress(snapshot);
    }));
    if (!settled && options.isCurrent()) timer = setTimeout(() => { void poll(); }, options.pollIntervalMs ?? 500);
  };
  timer = setTimeout(() => { void poll(); }, options.pollIntervalMs ?? 500);
  try { return await operation(); }
  finally { settled = true; if (timer) clearTimeout(timer); }
}

export function formatRuntimePreparation(snapshot: RuntimeToolSnapshot, t: TFunction): string {
  const stages: Record<RuntimeToolSnapshot['phase'], string> = {
    idle: 'common.runtimePreparation.checking', checking: 'common.runtimePreparation.checking',
    downloading: 'common.runtimePreparation.downloading', extracting: 'common.runtimePreparation.extracting',
    verifying: 'common.runtimePreparation.verifying', ready: 'common.runtimePreparation.ready',
    error: 'common.runtimePreparation.failed',
  };
  const stage = t(stages[snapshot.phase], { tool: { bun: 'Bun', node: 'Node.js', officecli: 'OfficeCLI' }[snapshot.tool] });
  if (snapshot.phase !== 'downloading') return stage;
  const received = (snapshot.received_bytes / 1_000_000).toFixed(1);
  return snapshot.total_bytes ? `${stage} ${received} / ${(snapshot.total_bytes / 1_000_000).toFixed(1)} MB` : `${stage} ${received} MB`;
}
