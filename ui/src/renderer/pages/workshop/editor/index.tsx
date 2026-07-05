/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Image editor entry point (M5 module).
 *
 * The canvas (M1/M7) opens the editor for an image node via
 * {@link openImageEditor}; the returned result is uploaded as new assets /
 * nodes by the caller. Types below are the frozen M0 contract; M5 replaces
 * this stub with the real modal implementation.
 */

export type ImageEditorMode = 'crop' | 'mask' | 'split' | 'upscale';

export interface ImageEditorRequest {
  mode: ImageEditorMode;
  /** Object URL / data URL of the source image (caller resolves via lib/media). */
  src: string;
  naturalWidth?: number;
  naturalHeight?: number;
}

export type ImageEditorResult =
  | { type: 'crop'; blob: Blob }
  /** Painted area is transparent (alpha 0) on an otherwise opaque copy. */
  | { type: 'mask'; maskBlob: Blob; prompt: string }
  | { type: 'split'; pieces: { blob: Blob; row: number; col: number }[] }
  | { type: 'upscale'; blob: Blob };

/**
 * Open the modal image editor. Resolves with the edit result, or `null` when
 * the user cancels. The M0 stub always resolves `null`.
 */
export async function openImageEditor(_req: ImageEditorRequest): Promise<ImageEditorResult | null> {
  return null;
}
