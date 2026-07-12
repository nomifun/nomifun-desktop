/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type { OcrModelServiceStatus } from '@/common/types/provider/ocrModelService';
import { useCallback, useRef, useState } from 'react';
import useSWR, { type KeyedMutator } from 'swr';

export const LOCAL_OCR_CATALOG_SWR_KEY = 'model-services/local/ocr/catalog';
export const LOCAL_OCR_STATUS_SWR_KEY = 'model-services/local/ocr/status';

const fetchCatalog = () => ipcBridge.managedModelService.local.ocr.catalog.invoke();
const fetchStatus = () => ipcBridge.managedModelService.local.ocr.status.invoke();

const hasActiveTransfer = (status: OcrModelServiceStatus | undefined): boolean =>
  Boolean(
    status?.models.some(
      (model) => model.installPhase === 'downloading' || model.installPhase === 'verifying'
    )
  );

/** Managed, opt-in OCR downloads with serialized lifecycle mutations. */
export const useLocalOcrModels = () => {
  const catalogQuery = useSWR(LOCAL_OCR_CATALOG_SWR_KEY, fetchCatalog, {
    revalidateOnFocus: false,
    revalidateOnReconnect: true,
    shouldRetryOnError: false,
  });
  const statusQuery = useSWR<OcrModelServiceStatus>(LOCAL_OCR_STATUS_SWR_KEY, fetchStatus, {
    revalidateOnFocus: false,
    revalidateOnReconnect: true,
    shouldRetryOnError: false,
    refreshInterval: (latestStatus) => (hasActiveTransfer(latestStatus) ? 1_000 : 10_000),
  });
  const [pendingAction, setPendingAction] = useState<string | null>(null);
  const pendingActionRef = useRef<string | null>(null);
  const mutateStatus: KeyedMutator<OcrModelServiceStatus> = statusQuery.mutate;

  const runAction = useCallback(
    async (key: string, action: () => Promise<OcrModelServiceStatus>) => {
      if (pendingActionRef.current) {
        throw new Error(`OCR model action already in progress: ${pendingActionRef.current}`);
      }
      pendingActionRef.current = key;
      setPendingAction(key);
      try {
        const status = await action();
        await mutateStatus(status, false);
        return status;
      } finally {
        pendingActionRef.current = null;
        setPendingAction(null);
      }
    },
    [mutateStatus]
  );

  const install = useCallback(
    (id: string) =>
      runAction(`install:${id}`, () => ipcBridge.managedModelService.local.ocr.install.invoke({ id })),
    [runAction]
  );

  const pause = useCallback(
    (id: string) =>
      runAction(`pause:${id}`, () => ipcBridge.managedModelService.local.ocr.pause.invoke({ id })),
    [runAction]
  );

  const resume = useCallback(
    (id: string) =>
      runAction(`resume:${id}`, () => ipcBridge.managedModelService.local.ocr.resume.invoke({ id })),
    [runAction]
  );

  const remove = useCallback(
    (id: string) =>
      runAction(`remove:${id}`, () => ipcBridge.managedModelService.local.ocr.remove.invoke({ id })),
    [runAction]
  );

  const refresh = useCallback(async () => {
    const [catalog, status] = await Promise.all([catalogQuery.mutate(), statusQuery.mutate()]);
    return { catalog, status };
  }, [catalogQuery, statusQuery]);

  return {
    catalog: catalogQuery.data,
    status: statusQuery.data,
    catalogError: catalogQuery.error,
    statusError: statusQuery.error,
    isLoading: catalogQuery.isLoading || statusQuery.isLoading,
    pendingAction,
    refresh,
    install,
    pause,
    resume,
    remove,
  };
};
