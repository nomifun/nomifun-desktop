/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 * Based on AionUi (https://github.com/iOfficeAI/AionUi)
 */

import { ipcBridge } from '@/common';
import type { CompareResult, FileChangeInfo, SnapshotInfo } from '@/common/types/platform/fileSnapshot';
import { useCallback, useEffect, useRef, useState } from 'react';

type UseFileChangesParams = {
  workspace: string;
  /**
   * Gate the snapshot init/dispose lifecycle. Defaults to `true` so conversation
   * callers (which don't pass it) are unaffected. When `false`, the mount effect
   * is a no-op until it flips back to `true`.
   */
  enabled?: boolean;
};

type UseFileChangesReturn = {
  staged: FileChangeInfo[];
  unstaged: FileChangeInfo[];
  changeCount: number;
  loading: boolean;
  snapshotInfo: SnapshotInfo | null;
  refreshChanges: () => Promise<void>;
  stageFile: (file_path: string) => Promise<void>;
  stageAll: () => Promise<void>;
  unstageFile: (file_path: string) => Promise<void>;
  unstageAll: () => Promise<void>;
  discardFile: (file_path: string, operation: FileChangeInfo['operation']) => Promise<void>;
  resetFile: (file_path: string, operation: FileChangeInfo['operation']) => Promise<void>;
};

export function useFileChanges({ workspace, enabled = true }: UseFileChangesParams): UseFileChangesReturn {
  const [result, setResult] = useState<CompareResult>({ staged: [], unstaged: [] });
  const [loading, setLoading] = useState(false);
  const [snapshotInfo, setSnapshotInfo] = useState<SnapshotInfo | null>(null);
  const initializedRef = useRef(false);

  useEffect(() => {
    if (!workspace || !enabled) return;

    initializedRef.current = false;
    setResult({ staged: [], unstaged: [] });
    setSnapshotInfo(null);

    ipcBridge.fileSnapshot.init
      .invoke({ workspace })
      .then((info) => {
        // Surface the mode/reason for consumers regardless of outcome.
        setSnapshotInfo(info);
        // `disabled` means the backend refused to track this workspace (drive
        // root, system dir, too large) and is NOT tracking it server-side. A
        // follow-up compare/getInfo would 400, so treat this as a terminal,
        // non-error state: leave `initializedRef` false so no compare fires.
        if (info.mode === 'disabled') return;
        initializedRef.current = true;
      })
      .catch((err) => {
        console.error('[useFileChanges] Failed to init snapshot:', err);
      });

    return () => {
      ipcBridge.fileSnapshot.dispose.invoke({ workspace }).catch(() => {});
    };
  }, [workspace, enabled]);

  // Silent refresh: update data without showing loading spinner (used after git operations)
  const silentRefresh = useCallback(async () => {
    if (!workspace || !initializedRef.current) return;
    try {
      const res = await ipcBridge.fileSnapshot.compare.invoke({ workspace });
      setResult(res);
    } catch (err) {
      console.error('[useFileChanges] Failed to compare:', err);
    }
  }, [workspace]);

  // Full refresh with loading indicator (used for manual refresh button)
  const refreshChanges = useCallback(async () => {
    if (!workspace || !initializedRef.current) return;
    setLoading(true);
    try {
      const res = await ipcBridge.fileSnapshot.compare.invoke({ workspace });
      setResult(res);
    } catch (err) {
      console.error('[useFileChanges] Failed to compare:', err);
    } finally {
      setLoading(false);
    }
  }, [workspace]);

  const stageFile = useCallback(
    async (file_path: string) => {
      if (!workspace) return;
      await ipcBridge.fileSnapshot.stageFile.invoke({ workspace, file_path });
      await silentRefresh();
    },
    [workspace, silentRefresh]
  );

  const stageAll = useCallback(async () => {
    if (!workspace) return;
    await ipcBridge.fileSnapshot.stageAll.invoke({ workspace });
    await silentRefresh();
  }, [workspace, silentRefresh]);

  const unstageFile = useCallback(
    async (file_path: string) => {
      if (!workspace) return;
      await ipcBridge.fileSnapshot.unstageFile.invoke({ workspace, file_path });
      await silentRefresh();
    },
    [workspace, silentRefresh]
  );

  const unstageAll = useCallback(async () => {
    if (!workspace) return;
    await ipcBridge.fileSnapshot.unstageAll.invoke({ workspace });
    await silentRefresh();
  }, [workspace, silentRefresh]);

  const discardFile = useCallback(
    async (file_path: string, operation: FileChangeInfo['operation']) => {
      if (!workspace) return;
      await ipcBridge.fileSnapshot.discardFile.invoke({ workspace, file_path, operation });
      await silentRefresh();
    },
    [workspace, silentRefresh]
  );

  const resetFile = useCallback(
    async (file_path: string, operation: FileChangeInfo['operation']) => {
      if (!workspace) return;
      await ipcBridge.fileSnapshot.resetFile.invoke({ workspace, file_path, operation });
      await silentRefresh();
    },
    [workspace, silentRefresh]
  );

  return {
    staged: result.staged,
    unstaged: result.unstaged,
    changeCount: result.staged.length + result.unstaged.length,
    loading,
    snapshotInfo,
    refreshChanges,
    stageFile,
    stageAll,
    unstageFile,
    unstageAll,
    discardFile,
    resetFile,
  };
}
