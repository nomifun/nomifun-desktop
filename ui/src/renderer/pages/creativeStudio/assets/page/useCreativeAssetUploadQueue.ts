/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useCallback, useEffect, useReducer, useRef } from 'react';
import type { TFunction } from 'i18next';
import { useTranslation } from 'react-i18next';

import { CreativeAssetUploadError } from '../api';
import type { CreativeAssetUploadItem } from '../components';
import type { UseCreativeAssetsResult } from '../useCreativeAssets';
import { creativeAssetUploadQueueReducer } from './uploadQueue';

let uploadSequence = 0;

const nextUploadId = (): string => {
  const uuid = globalThis.crypto?.randomUUID?.();
  if (uuid) return `creative-upload-${uuid}`;
  uploadSequence += 1;
  return `creative-upload-${Date.now().toString(36)}-${uploadSequence.toString(36)}`;
};

const uploadErrorText = (reason: unknown, t: TFunction): string => {
  if (reason instanceof CreativeAssetUploadError) {
    if (reason.code === 'aborted') {
      return t('creativeStudio.assets.upload.canceled', { defaultValue: '上传已取消。' });
    }
    if (reason.code === 'too_large') {
      return t('creativeStudio.assets.upload.backendLimitExceeded', {
        defaultValue: '素材超过后端 64 MB 上传限制。',
      });
    }
    if (reason.code === 'network') {
      return t('creativeStudio.assets.upload.backendUnreachable', {
        defaultValue: '素材上传失败：无法连接后端服务。',
      });
    }
    if (reason.code === 'invalid_response') {
      return t('creativeStudio.assets.upload.invalidResponse', {
        defaultValue: '素材上传失败：后端返回了无效响应。',
      });
    }
    if (reason.code === 'http') {
      return t('creativeStudio.assets.upload.failed', {
        defaultValue: '素材上传失败：{{reason}}',
        reason: reason.message.replace(/^Asset upload failed:\s*/i, ''),
      });
    }
    return reason.message;
  }
  return reason instanceof Error ? reason.message : String(reason);
};

export interface UseCreativeAssetUploadQueueResult {
  items: readonly CreativeAssetUploadItem[];
  start(files: readonly File[]): void;
  cancel(uploadId: string): void;
  retry(uploadId: string): void;
  dismiss(uploadId: string): void;
}

export function useCreativeAssetUploadQueue(
  upload: UseCreativeAssetsResult['upload']
): UseCreativeAssetUploadQueueResult {
  const { t } = useTranslation();
  const [items, dispatch] = useReducer(creativeAssetUploadQueueReducer, []);
  const filesRef = useRef(new Map<string, File>());
  const controllersRef = useRef(new Map<string, AbortController>());
  const mountedRef = useRef(true);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      for (const controller of controllersRef.current.values()) controller.abort();
      controllersRef.current.clear();
    };
  }, []);

  const run = useCallback((uploadId: string, file: File): void => {
    const controller = new AbortController();
    controllersRef.current.set(uploadId, controller);
    void upload(
      file,
      { title: file.name, inLibrary: true },
      controller.signal,
      (percent) => {
        if (mountedRef.current) dispatch({ type: 'progress', id: uploadId, percent });
      }
    )
      .then(() => {
        if (mountedRef.current) dispatch({ type: 'complete', id: uploadId });
      })
      .catch((reason: unknown) => {
        if (mountedRef.current) {
          dispatch({ type: 'fail', id: uploadId, error: uploadErrorText(reason, t) });
        }
      })
      .finally(() => {
        controllersRef.current.delete(uploadId);
      });
  }, [t, upload]);

  const start = useCallback((files: readonly File[]): void => {
    for (const file of files) {
      const id = nextUploadId();
      filesRef.current.set(id, file);
      dispatch({
        type: 'enqueue',
        item: { id, fileName: file.name, percent: 0, status: 'uploading' },
      });
      run(id, file);
    }
  }, [run]);

  const cancel = useCallback((uploadId: string): void => {
    controllersRef.current.get(uploadId)?.abort();
  }, []);

  const retry = useCallback((uploadId: string): void => {
    const file = filesRef.current.get(uploadId);
    if (!file || controllersRef.current.has(uploadId)) return;
    dispatch({ type: 'restart', id: uploadId });
    run(uploadId, file);
  }, [run]);

  const dismiss = useCallback((uploadId: string): void => {
    if (controllersRef.current.has(uploadId)) return;
    filesRef.current.delete(uploadId);
    dispatch({ type: 'dismiss', id: uploadId });
  }, []);

  return { items, start, cancel, retry, dismiss };
}
