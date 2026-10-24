/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { httpRequest, isBackendHttpError } from './httpBridge';

type WorkspaceEntry = { name: string; type: string };
const BUSY_RETRY_DELAYS_MS = [250, 500, 1000];

function waitForRetry(delay: number, signal?: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    const cancelled = () => {
      clearTimeout(timer);
      signal?.removeEventListener('abort', cancelled);
      reject(new DOMException('Workspace read cancelled', 'AbortError'));
    };
    const timer = setTimeout(() => {
      signal?.removeEventListener('abort', cancelled);
      resolve();
    }, delay);
    if (signal?.aborted) cancelled();
    else signal?.addEventListener('abort', cancelled, { once: true });
  });
}

/** A lifecycle writer may briefly own this read's Session; other refusals keep their meaning. */
export async function readConversationWorkspaceEntries(path: string, signal?: AbortSignal): Promise<WorkspaceEntry[]> {
  for (let attempt = 0; ; attempt++) {
    try {
      return await httpRequest<WorkspaceEntry[]>('GET', path, undefined, { signal });
    } catch (error) {
      if (!isBackendHttpError(error) || error.status !== 409 || error.code !== 'WORKSPACE_LIFECYCLE_BUSY'
        || attempt >= BUSY_RETRY_DELAYS_MS.length) throw error;
      await waitForRetry(BUSY_RETRY_DELAYS_MS[attempt], signal);
    }
  }
}
