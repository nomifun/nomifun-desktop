/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Main-thread acquisition of the MODNet matting model.
 *
 * Root-cause fix for the DIY figure "根本用不了" bug: the model used to be
 * lazy-downloaded from huggingface.co **inside the matting worker**, wrapped in
 * a 30 s timeout that also covered the 25 MB transfer — so the first attempt
 * always timed out, fell back to heuristic matting, and dead-ended real photos.
 *
 * Now the download happens here, on the main thread:
 *   - fetched from the **local backend proxy** (`/api/companion/matting-model`), which
 *     mirror-downloads + disk-caches it once (reachable even behind the GFW),
 *   - with progress + **no hard timeout** (a slow transfer is allowed to finish),
 *   - mirrored into Cache Storage so the worker reads a local copy (no auth, no
 *     network) and repeat sessions skip the localhost round-trip entirely.
 *
 * `ensureMattingModel` is single-flight: concurrent callers (prewarm on wizard
 * open + the await before matting) share one in-flight download.
 */

import { getBaseUrl } from '@/common/adapter/httpBridge';
import { MODEL_CACHE_NAME, modelCacheKey } from './modelCacheKey';

export type ModelProgress = (loaded: number, total: number) => void;

/** Sanity floor mirroring the backend — never treat an error page as the model. */
const MIN_VALID_BYTES = 8 * 1024 * 1024;

let inFlight: Promise<void> | null = null;
const progressSubs = new Set<ModelProgress>();

async function openCache(): Promise<Cache | undefined> {
  if (typeof caches === 'undefined') return undefined;
  try {
    return await caches.open(MODEL_CACHE_NAME);
  } catch {
    return undefined;
  }
}

async function cachedModelOk(cache: Cache | undefined): Promise<boolean> {
  if (!cache) return false;
  try {
    const hit = await cache.match(modelCacheKey());
    if (!hit) return false;
    const len = Number(hit.headers.get('content-length') ?? 0);
    if (len && len >= MIN_VALID_BYTES) return true;
    // No/!plausible length header — verify by reading the body size once.
    const buf = await hit.arrayBuffer();
    return buf.byteLength >= MIN_VALID_BYTES;
  } catch {
    return false;
  }
}

async function downloadToCache(cache: Cache | undefined): Promise<void> {
  const url = `${getBaseUrl()}/api/companion/matting-model`;
  const res = await fetch(url);
  if (!res.ok) throw new Error(`matting model fetch failed: HTTP ${res.status}`);
  const total = Number(res.headers.get('content-length') ?? 0) || 0;

  // Typed as ArrayBuffer-backed (both assignments below build it via
  // `new Uint8Array(number | ArrayBuffer)`), so `bytes.buffer` is a concrete
  // `ArrayBuffer` and a valid `BodyInit` — lib.es2024 rejects the generic
  // `Uint8Array<ArrayBufferLike>` view.
  let bytes: Uint8Array<ArrayBuffer>;
  if (res.body) {
    const reader = res.body.getReader();
    const chunks: Uint8Array[] = [];
    let loaded = 0;
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      chunks.push(value);
      loaded += value.byteLength;
      for (const sub of progressSubs) sub(loaded, total);
    }
    bytes = new Uint8Array(loaded);
    let offset = 0;
    for (const chunk of chunks) {
      bytes.set(chunk, offset);
      offset += chunk.byteLength;
    }
  } else {
    const buf = await res.arrayBuffer();
    bytes = new Uint8Array(buf);
    for (const sub of progressSubs) sub(bytes.byteLength, total || bytes.byteLength);
  }

  if (bytes.byteLength < MIN_VALID_BYTES) {
    throw new Error(`matting model too small (${bytes.byteLength} bytes)`);
  }

  if (cache) {
    try {
      await cache.put(
        modelCacheKey(),
        // `bytes` is freshly built via `new Uint8Array(...)` above (full-span,
        // ArrayBuffer-backed), so `bytes.buffer` is the identical byte range —
        // and is a concrete `ArrayBuffer` (a valid `BodyInit`), unlike the
        // `Uint8Array<ArrayBufferLike>` view which lib.es2024 rejects.
        new Response(bytes.buffer, {
          headers: {
            'Content-Type': 'application/octet-stream',
            'Content-Length': String(bytes.byteLength),
          },
        })
      );
    } catch {
      // Quota/eviction — the worker can still fetch from the backend next time.
    }
  }
}

/**
 * Resolve once the matting model is present in Cache Storage (downloading via
 * the backend proxy on first use). Rejects only if the model cannot be obtained
 * at all (offline + nothing cached) — callers degrade to heuristic matting.
 *
 * @param onProgress receives (loaded, total) bytes during an active download.
 */
export function ensureMattingModel(onProgress?: ModelProgress): Promise<void> {
  if (onProgress) progressSubs.add(onProgress);
  if (!inFlight) {
    inFlight = (async () => {
      const cache = await openCache();
      if (await cachedModelOk(cache)) return;
      await downloadToCache(cache);
    })();
    // A failed attempt must be retryable (transient offline), so clear the
    // memo on rejection; keep it on success so we never re-download.
    inFlight.catch(() => {
      inFlight = null;
    });
  }
  const p = inFlight;
  if (onProgress) {
    void p.finally(() => progressSubs.delete(onProgress));
  }
  return p;
}

/** True if the model is already cached (cheap check for UI hints). */
export async function isMattingModelReady(): Promise<boolean> {
  return cachedModelOk(await openCache());
}
