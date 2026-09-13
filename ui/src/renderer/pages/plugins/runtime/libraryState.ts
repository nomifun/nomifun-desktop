import { useCallback, useEffect, useState } from 'react';
import { ipcBridge } from '@/common';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import {
  pluginRuntimeProduct,
  type PluginRuntimeDraft,
  type PluginRuntimeWorkspace,
  type PluginRuntimeLibraryItem,
} from '@/common/adapter/pluginRuntimeProductBridge';
import type { PluginRuntimeSummary } from '@/common/types/pluginRuntimePlatform';

export const MINIAPP_LIBRARY_CHANGED = 'nomifun:plugin-library-changed';
export const emptyItem = (): PluginRuntimeLibraryItem => ({
  collection_id: null,
  pinned: false,
  last_opened: 0,
  name: null,
});
export const libraryChanged = () =>
  window.dispatchEvent(new Event(MINIAPP_LIBRARY_CHANGED));
let mutationQueue: Promise<unknown> = Promise.resolve();

/** Serialize local writes and reapply the user's operation to authoritative state after CAS conflicts. */
export function updatePluginRuntimeWorkspace(
  update: (workspace: PluginRuntimeWorkspace) => void,
): Promise<PluginRuntimeWorkspace> {
  const operation = mutationQueue
    .catch(() => undefined)
    .then(async () => {
      for (let attempt = 0; attempt < 3; attempt++) {
        const workspace = await pluginRuntimeProduct.workspace.invoke();
        update(workspace);
        try {
          const next = await pluginRuntimeProduct.updateWorkspace.invoke(workspace);
          libraryChanged();
          return next;
        } catch (error) {
          if (
            !isBackendHttpError(error) ||
            error.status !== 409 ||
            attempt === 2
          )
            throw error;
        }
      }
      throw new Error('PluginRuntime collection could not be saved');
    });
  mutationQueue = operation;
  return operation;
}

export function usePluginRuntimeLibrary() {
  const [apps, setApps] = useState<PluginRuntimeSummary[]>([]);
  const [drafts, setDrafts] = useState<PluginRuntimeDraft[]>([]);
  const [workspace, setWorkspace] = useState<PluginRuntimeWorkspace>({
    revision: 0,
    collections: [],
    items: {},
  });
  const [loading, setLoading] = useState(true);
  const [failed, setFailed] = useState(false);
  const refresh = useCallback(async () => {
    try {
      const [library, organization, pending] = await Promise.all([
        ipcBridge.pluginRuntimes.library.invoke(),
        pluginRuntimeProduct.workspace.invoke(),
        pluginRuntimeProduct.drafts.invoke(),
      ]);
      setApps(library.plugins);
      setWorkspace(organization);
      setDrafts(pending);
      setFailed(false);
    } catch {
      setFailed(true);
    } finally {
      setLoading(false);
    }
  }, []);
  useEffect(() => {
    void refresh();
    window.addEventListener(MINIAPP_LIBRARY_CHANGED, refresh);
    return () => window.removeEventListener(MINIAPP_LIBRARY_CHANGED, refresh);
  }, [refresh]);
  return { apps, drafts, workspace, loading, failed, refresh };
}
