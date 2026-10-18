/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { httpBlobRequest } from '@/common/adapter/httpBridge';
import model from '../../../../../crates/backend/nomifun-companion/matting-model.json';

export type ModelProgress = (loaded: number, total: number) => void;

async function downloadModel(onProgress: ModelProgress): Promise<Blob> {
  // The backend verifies the approved digest before serving these exact bytes.
  // The requested identity rejects frontend/backend version drift. Browser
  // storage and secure-context-only Web Crypto are not prerequisites for WebUI.
  const blob = await httpBlobRequest(`/api/companion/matting-model?sha256=${model.sha256}`, {
    maxBytes: model.size,
    onProgress,
  });
  if (blob.size !== model.size) {
    throw new Error(`Matting model ${model.id}: expected ${model.size} bytes, received ${blob.size}`);
  }
  return blob;
}

/**
 * One immutable, verified model per renderer lifetime. The backend owns the
 * durable cache; callers receive actual bytes rather than a browser-cache
 * readiness flag. A Blob can be cloned into a Worker without detaching this
 * shared copy. Failed acquisition is retryable and never memoized as ready.
 */
export function createMattingModelLoader(download = downloadModel) {
  let flight: Promise<Blob> | undefined;
  const subscribers = new Set<ModelProgress>();
  const publish: ModelProgress = (loaded, total) => {
    for (const subscriber of subscribers) {
      try {
        subscriber(loaded, total);
      } catch {
        // A disposed view must not fail a download shared with other callers.
      }
    }
  };

  return (onProgress?: ModelProgress, signal?: AbortSignal): Promise<Blob> => {
    if (signal?.aborted) return Promise.reject(new DOMException('Request cancelled', 'AbortError'));
    if (onProgress) subscribers.add(onProgress);
    if (!flight) {
      const pending = Promise.resolve().then(() => download(publish));
      flight = pending;
      void pending.catch(() => {
        if (flight === pending) flight = undefined;
      });
    }
    const current = flight;
    // Cancellation belongs to this consumer. Background preparation and other
    // windows/views can still finish the shared acquisition.
    return new Promise<Blob>((resolve, reject) => {
      const detach = () => {
        if (onProgress) subscribers.delete(onProgress);
        signal?.removeEventListener('abort', abort);
      };
      const abort = () => {
        detach();
        reject(new DOMException('Request cancelled', 'AbortError'));
      };
      signal?.addEventListener('abort', abort, { once: true });
      void current.then(
        blob => { detach(); resolve(blob); },
        error => { detach(); reject(error); }
      );
    });
  };
}

export const ensureMattingModel = createMattingModelLoader();
