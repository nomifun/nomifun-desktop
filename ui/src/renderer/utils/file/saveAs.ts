/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { buildBackendAuthHeaders } from '@/common/adapter/httpBridge';
import { isTauriRuntime } from '@/common/adapter/tauriRuntime';

export interface SaveAsOptions {
  suggestedName: string;
  mimeType?: string | null;
  dialogTitle?: string;
}

export type SaveAsResult =
  | { status: 'saved'; path?: string }
  | { status: 'cancelled' };

type BlobSource = Blob | (() => Blob | Promise<Blob>);

interface BrowserWritableFile {
  write(data: Blob): Promise<void>;
  close(): Promise<void>;
}

interface BrowserFileHandle {
  createWritable(): Promise<BrowserWritableFile>;
}

type BrowserSaveFilePicker = (options: {
  suggestedName: string;
  types?: Array<{
    description: string;
    accept: Record<string, string[]>;
  }>;
}) => Promise<BrowserFileHandle>;

const cleanSuggestedName = (name: string): string =>
  name
    .trim()
    .replace(/[\\/:*?"<>|\u0000-\u001f]+/g, '-')
    .replace(/[. ]+$/g, '') || 'download';

const fileExtension = (name: string): string | null => {
  const match = /\.([a-z0-9]{1,16})$/i.exec(name);
  return match?.[1]?.toLocaleLowerCase() ?? null;
};

const resolveBlob = async (source: BlobSource): Promise<Blob> =>
  typeof source === 'function' ? await source() : source;

const isPickerCancellation = (error: unknown): boolean =>
  error instanceof DOMException && error.name === 'AbortError';

const saveWithTauri = async (
  source: BlobSource,
  options: SaveAsOptions,
  suggestedName: string
): Promise<SaveAsResult> => {
  const [{ save }, { downloadDir, join }] = await Promise.all([
    import('@tauri-apps/plugin-dialog'),
    import('@tauri-apps/api/path'),
  ]);
  let defaultPath = suggestedName;
  try {
    defaultPath = await join(await downloadDir(), suggestedName);
  } catch {
    // The native dialog can still provide a useful file-name default when an
    // operating system does not expose a Downloads directory.
  }
  const extension = fileExtension(suggestedName);
  const path = await save({
    title: options.dialogTitle,
    defaultPath,
    filters: extension
      ? [{ name: options.mimeType?.trim() || extension.toLocaleUpperCase(), extensions: [extension] }]
      : undefined,
  });
  if (!path) return { status: 'cancelled' };

  // Resolve remote data only after the user has picked a destination. This
  // avoids an unnecessary transfer when the Save As dialog is cancelled.
  const blob = await resolveBlob(source);
  const { writeFile } = await import('@tauri-apps/plugin-fs');
  await writeFile(path, new Uint8Array(await blob.arrayBuffer()));
  return { status: 'saved', path };
};

const saveWithBrowserPicker = async (
  picker: BrowserSaveFilePicker,
  source: BlobSource,
  options: SaveAsOptions,
  suggestedName: string
): Promise<SaveAsResult> => {
  const extension = fileExtension(suggestedName);
  try {
    // Open the picker before any asynchronous fetch so the browser still sees
    // this call as part of the user's click gesture.
    const handle = await picker({
      suggestedName,
      types: extension
        ? [{
            description: options.mimeType?.trim() || extension.toLocaleUpperCase(),
            accept: {
              [options.mimeType?.trim() || 'application/octet-stream']: [`.${extension}`],
            },
          }]
        : undefined,
    });
    const writable = await handle.createWritable();
    await writable.write(await resolveBlob(source));
    await writable.close();
    return { status: 'saved' };
  } catch (error) {
    if (isPickerCancellation(error)) return { status: 'cancelled' };
    throw error;
  }
};

const saveWithDownloadFallback = async (
  source: BlobSource,
  suggestedName: string
): Promise<SaveAsResult> => {
  const blob = await resolveBlob(source);
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement('a');
  anchor.href = url;
  anchor.download = suggestedName;
  anchor.rel = 'noopener noreferrer';
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  window.setTimeout(() => URL.revokeObjectURL(url), 1_000);
  return { status: 'saved' };
};

/** Show a Save As destination before writing or downloading the file bytes. */
export async function saveBlobAs(
  source: BlobSource,
  options: SaveAsOptions
): Promise<SaveAsResult> {
  const suggestedName = cleanSuggestedName(options.suggestedName);
  if (isTauriRuntime()) {
    return saveWithTauri(source, options, suggestedName);
  }

  const picker = (window as Window & {
    showSaveFilePicker?: BrowserSaveFilePicker;
  }).showSaveFilePicker;
  if (picker) {
    return saveWithBrowserPicker(picker.bind(window), source, options, suggestedName);
  }

  // Firefox and older desktop browsers do not expose showSaveFilePicker.
  // A blob URL still guarantees that cross-origin media is saved instead of
  // replacing the current page, while the browser owns its download location.
  return saveWithDownloadFallback(source, suggestedName);
}

/** Fetch a protected backend URL lazily, after the destination is selected. */
export function saveUrlAs(url: string, options: SaveAsOptions): Promise<SaveAsResult> {
  return saveBlobAs(async () => {
    const response = await fetch(url, {
      method: 'GET',
      headers: buildBackendAuthHeaders('GET'),
      credentials: 'same-origin',
      cache: 'no-store',
    });
    if (!response.ok) {
      throw new Error(`Unable to load the file (${response.status})`);
    }
    return response.blob();
  }, options);
}
