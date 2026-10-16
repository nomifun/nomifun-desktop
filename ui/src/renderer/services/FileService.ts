/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { buildBackendAuthHeaders, getBaseUrl } from '@/common/adapter/httpBridge';
import { ipcBridge } from '@/common';
import { trackUpload, type UploadSource } from '@/renderer/hooks/file/useUploadState';
import type { ConversationId } from '@/common/types/ids';

/** Sentinel error message used when an upload is cancelled by the caller. */
export const UPLOAD_ABORTED_ERROR = 'Upload aborted';

export interface UploadFileOptions {
  /** Cancel the upload from the outside. Closing the XHR also frees the backend connection. */
  signal?: AbortSignal;
}

/**
 * Upload a file to the backend via HTTP multipart.
 *
 * Works in both the Tauri desktop shell (via `http://127.0.0.1:<backendPort>`) and WebUI
 * (same-origin reverse-proxied). The backend stages uploads in its temporary
 * directory, isolated by conversation ID or in the general staging directory.
 *
 * Field names match the backend contract exactly (snake_case): `file`,
 * `file_name` (optional), `conversation_id` (optional). The response is
 * `ApiResponse<String>` where `data` is the absolute file path on disk.
 *
 * @param onProgress Optional callback receiving upload percentage (0-100).
 * @param options    Optional bag — currently supports an `AbortSignal` so callers can cancel.
 */
export async function uploadFileViaHttp(
  file: File,
  conversation_id?: ConversationId,
  onProgress?: (percent: number) => void,
  file_name?: string,
  options?: UploadFileOptions
): Promise<string> {
  const formData = new FormData();
  formData.append('file', file);
  if (file_name) {
    formData.append('file_name', file_name);
  }
  if (conversation_id) {
    formData.append('conversation_id', conversation_id);
  }

  return new Promise<string>((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    xhr.open('POST', `${getBaseUrl()}/api/fs/upload`);

    // This is a raw XHR (for upload-progress events), so it bypasses both
    // `httpRequest`'s header logic and the desktop shell's `window.fetch`
    // interceptor. Apply the trust (desktop) / CSRF (WebUI) headers ourselves,
    // or the TrustLocalToken-guarded upload endpoint rejects it with 403.
    for (const [name, value] of Object.entries(buildBackendAuthHeaders('POST'))) {
      xhr.setRequestHeader(name, value);
    }

    // Wire AbortSignal → xhr.abort. Closing the XHR tears down the underlying
    // socket; the backend (axum/multer) treats the truncated multipart body as
    // a client disconnect and stops reading. No explicit cancel IPC needed.
    const signal = options?.signal;
    let onSignalAbort: (() => void) | null = null;
    if (signal) {
      if (signal.aborted) {
        // Caller asked to abort before send — bail out without opening a socket.
        reject(new Error(UPLOAD_ABORTED_ERROR));
        return;
      }
      onSignalAbort = () => {
        try {
          xhr.abort();
        } catch {
          /* ignore */
        }
      };
      signal.addEventListener('abort', onSignalAbort);
    }

    const detachSignal = (): void => {
      if (signal && onSignalAbort) {
        signal.removeEventListener('abort', onSignalAbort);
        onSignalAbort = null;
      }
    };

    if (onProgress) {
      xhr.upload.addEventListener('progress', (e) => {
        if (e.lengthComputable) {
          onProgress(Math.round((e.loaded / e.total) * 100));
        }
      });
    }

    xhr.addEventListener('load', () => {
      detachSignal();
      if (xhr.status === 413) {
        reject(new Error('FILE_TOO_LARGE'));
        return;
      }
      if (xhr.status < 200 || xhr.status >= 300) {
        reject(new Error(`Upload failed: ${xhr.status} ${xhr.statusText}`));
        return;
      }
      try {
        const result = JSON.parse(xhr.responseText) as { success: boolean; data?: string };
        if (!result.success || typeof result.data !== 'string' || !result.data) {
          reject(new Error('Upload failed: server returned unsuccessful response'));
        } else {
          resolve(result.data);
        }
      } catch {
        reject(new Error('Upload failed: invalid server response'));
      }
    });

    xhr.addEventListener('error', () => {
      detachSignal();
      reject(new Error('Upload failed: network error'));
    });

    xhr.addEventListener('abort', () => {
      detachSignal();
      reject(new Error(UPLOAD_ABORTED_ERROR));
    });

    xhr.send(formData);
  });
}
// Clipboard attachment filters shared by the conversation and guide composers.

/** 支持的图片文件扩展名 */
export const imageExts = ['.jpg', '.jpeg', '.png', '.gif', '.bmp', '.webp', '.svg'];

/** 支持的文档文件扩展名 */
const documentExts = ['.pdf', '.doc', '.docx', '.pptx', '.xlsx', '.odt', '.odp', '.ods'];

/** 支持的文本文件扩展名 */
const textExts = [
  '.txt',
  '.md',
  '.json',
  '.xml',
  '.csv',
  '.log',
  '.js',
  '.ts',
  '.jsx',
  '.tsx',
  '.html',
  '.css',
  '.scss',
  '.py',
  '.java',
  '.cpp',
  '.c',
  '.h',
  '.go',
  '.rs',
  '.yml',
  '.yaml',
  '.toml',
  '.ini',
  '.conf',
  '.config',
];

/** File extensions accepted by composers that enable clipboard filtering. */
export const allSupportedExts = [...imageExts, ...documentExts, ...textExts];

// 文件元数据接口
export interface FileMetadata {
  name: string;
  path: string;
  size: number;
  type: string;
  lastModified: number;
}

// 获取文件扩展名
export function getFileExtension(file_name: string): string {
  const lastDotIndex = file_name.lastIndexOf('.');
  return lastDotIndex > -1 ? file_name.substring(lastDotIndex).toLowerCase() : '';
}

class FileServiceClass {
  /** Native drops keep the same host paths as the desktop file picker. */
  async processNativePaths(paths: readonly string[]): Promise<FileMetadata[]> {
    const files: FileMetadata[] = [];
    for (const path of new Set(paths)) {
      try {
        // The native event is an explicit file selection, just like the picker.
        // Scope this inspection to its parent so files on other drives work too.
        const parentEnd = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'));
        const metadata = await ipcBridge.fs.getFileMetadata.invoke({ path,
          ...(parentEnd >= 0 ? { workspace: path.slice(0, parentEnd + 1) } : {}) });
        // The '+' picker accepts files, so dropping a folder must not attach it.
        if (!metadata.isDirectory) {
          files.push({ name: metadata.name, path, size: metadata.size,
            type: metadata.type, lastModified: metadata.lastModified });
        }
      } catch (error) {
        console.warn('Failed to inspect dropped file:', path, error);
      }
    }
    return files;
  }

  /**
   * Upload browser File objects from drops and file inputs via HTTP multipart.
   * Native desktop selections use explicit host paths through processNativePaths.
   */
  async processDroppedFiles(
    files: FileList,
    conversation_id?: ConversationId,
    source: UploadSource = 'sendbox'
  ): Promise<FileMetadata[]> {
    const processedFiles: FileMetadata[] = [];

    for (let i = 0; i < files.length; i++) {
      const file = files[i];
      // Each upload owns its own AbortController; cancellation and conversation
      // switches use the same tracked upload lifecycle.
      const controller = new AbortController();
      const tracker = trackUpload(file.size, {
        source,
        name: file.name,
        conversationId: conversation_id,
        onAbort: () => controller.abort(),
      });
      try {
        const file_path = await uploadFileViaHttp(file, conversation_id, tracker.onProgress, undefined, {
          signal: controller.signal,
        });
        processedFiles.push({
          name: file.name,
          path: file_path,
          size: file.size,
          type: file.type,
          lastModified: file.lastModified,
        });
      } catch (error) {
        // Re-throw size errors so caller can show user-facing toast.
        if (error instanceof Error && error.message === 'FILE_TOO_LARGE') {
          throw error;
        }
        if (!(error instanceof Error && error.message === UPLOAD_ABORTED_ERROR)) {
          console.error('Failed to upload browser file:', error);
        }
      } finally {
        tracker.finish();
      }
    }

    return processedFiles;
  }
}

export const FileService = new FileServiceClass();
