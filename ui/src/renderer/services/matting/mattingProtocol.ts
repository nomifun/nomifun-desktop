/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/** The caller owns this deadline: terminating the worker also stops synchronous WASM. */
export const ML_TIMEOUT_MS = 30_000;

export interface MatteRequest {
  id: number;
  type: 'matte';
  bitmap: ImageBitmap;
  mode: 'auto' | 'heuristic';
  tolerance?: number;
  /** Verified model bytes, structured-cloned with the request (never transferred). */
  model?: Blob;
}

export type MatteMethod = 'passthrough' | 'ml' | 'heuristic';

export type MatteFallbackReason =
  | 'model_unavailable'
  | 'inference_failed'
  | 'inference_timeout'
  | 'unsuitable_image';

export type MatteResponse =
  | {
      id: number;
      type: 'progress';
      phase: 'download' | 'infer' | 'process';
      loaded?: number;
      total?: number;
    }
  | {
      id: number;
      type: 'fallback';
      reason: MatteFallbackReason;
      /** Diagnostic information only; render a localized message for the reason. */
      detail?: string;
    }
  | {
      id: number;
      type: 'done';
      blob: Blob;
      aspect: number;
      headBox: { x: number; y: number; w: number };
      method: MatteMethod;
    }
  | { id: number; type: 'error'; message: string };
