/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Media utilities for the canvas: reading image dimensions and picking local
 * files. (Asset → object-URL resolution lives in the shared
 * `assets/useWorkshopMedia.ts` hook.)
 */

/** Read an image file's natural dimensions (best-effort; resolves null on failure). */
export function readImageSize(file: File | Blob): Promise<{ width: number; height: number } | null> {
  return new Promise((resolve) => {
    const url = URL.createObjectURL(file);
    const img = new Image();
    img.onload = () => {
      const size = { width: img.naturalWidth, height: img.naturalHeight };
      URL.revokeObjectURL(url);
      resolve(size.width > 0 && size.height > 0 ? size : null);
    };
    img.onerror = () => {
      URL.revokeObjectURL(url);
      resolve(null);
    };
    img.src = url;
  });
}

export function isImageFile(file: { type?: string; name?: string }): boolean {
  if (file.type) return file.type.startsWith('image/');
  return /\.(png|jpe?g|gif|webp|bmp|svg|avif)$/i.test(file.name ?? '');
}

export function isVideoFile(file: { type?: string; name?: string }): boolean {
  if (file.type) return file.type.startsWith('video/');
  return /\.(mp4|webm|mov|mkv|avi|m4v)$/i.test(file.name ?? '');
}

/** Open a native file picker and resolve the chosen files (empty if cancelled). */
export function pickFiles(accept: string, multiple = false): Promise<File[]> {
  return new Promise((resolve) => {
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = accept;
    input.multiple = multiple;
    input.style.position = 'fixed';
    input.style.left = '-9999px';
    let settled = false;
    const done = (files: File[]): void => {
      if (settled) return;
      settled = true;
      window.removeEventListener('focus', onFocus, true);
      input.remove();
      resolve(files);
    };
    input.addEventListener('change', () => done(input.files ? Array.from(input.files) : []));
    // Fallback: if the dialog is dismissed, `change` never fires — resolve empty
    // shortly after the window regains focus.
    const onFocus = (): void => {
      window.setTimeout(() => done([]), 400);
    };
    window.addEventListener('focus', onFocus, true);
    document.body.appendChild(input);
    input.click();
  });
}
