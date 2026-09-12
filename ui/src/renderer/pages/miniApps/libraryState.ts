import { useCallback, useEffect, useState } from 'react';
import { ipcBridge } from '@/common';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import {
  miniAppProduct,
  type MiniAppDraft,
  type MiniAppWorkspace,
  type MiniAppLibraryItem,
} from '@/common/adapter/miniAppProductBridge';
import type { MiniAppSummary } from '@/common/types/miniAppPlatform';

export const MINIAPP_LIBRARY_CHANGED = 'nomifun:miniapp-library-changed';
export const emptyItem = (): MiniAppLibraryItem => ({
  collection_id: null,
  pinned: false,
  last_opened: 0,
  name: null,
});
export const libraryChanged = () =>
  window.dispatchEvent(new Event(MINIAPP_LIBRARY_CHANGED));
let mutationQueue: Promise<unknown> = Promise.resolve();

/** Serialize local writes and reapply the user's operation to authoritative state after CAS conflicts. */
export function updateMiniAppWorkspace(
  update: (workspace: MiniAppWorkspace) => void,
): Promise<MiniAppWorkspace> {
  const operation = mutationQueue
    .catch(() => undefined)
    .then(async () => {
      for (let attempt = 0; attempt < 3; attempt++) {
        const workspace = await miniAppProduct.workspace.invoke();
        update(workspace);
        try {
          const next = await miniAppProduct.updateWorkspace.invoke(workspace);
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
      throw new Error('MiniApp collection could not be saved');
    });
  mutationQueue = operation;
  return operation;
}

export function useMiniAppLibrary() {
  const [apps, setApps] = useState<MiniAppSummary[]>([]);
  const [drafts, setDrafts] = useState<MiniAppDraft[]>([]);
  const [workspace, setWorkspace] = useState<MiniAppWorkspace>({
    revision: 0,
    collections: [],
    items: {},
  });
  const [loading, setLoading] = useState(true);
  const [failed, setFailed] = useState(false);
  const refresh = useCallback(async () => {
    try {
      const [library, organization, pending] = await Promise.all([
        ipcBridge.miniapps.library.invoke(),
        miniAppProduct.workspace.invoke(),
        miniAppProduct.drafts.invoke(),
      ]);
      setApps(library.miniapps);
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

export type LibraryFilter =
  | 'all'
  | 'recent'
  | 'pinned'
  | 'drafts'
  | 'unfiled'
  | 'trash'
  | string;
export function selectLibraryApps(
  apps: MiniAppSummary[],
  workspace: MiniAppWorkspace,
  filter: LibraryFilter,
  query: string,
  sort: string,
): MiniAppSummary[] {
  const text = query.trim().toLocaleLowerCase();
  return apps
    .filter((app) => {
      if (
        filter === 'trash'
          ? app.lifecycle !== 'trashed' && app.lifecycle !== 'deleting'
          : app.lifecycle === 'trashed' ||
            app.lifecycle === 'deleting' ||
            !app.releases.active
      )
        return false;
      const item = workspace.items[app.miniapp_id] ?? emptyItem();
      if (
        (filter === 'pinned' && !item.pinned) ||
        (filter === 'recent' && !item.last_opened) ||
        (filter === 'unfiled' && item.collection_id)
      )
        return false;
      if (
        !['all', 'recent', 'pinned', 'unfiled', 'trash'].includes(filter) &&
        item.collection_id !== filter
      )
        return false;
      return (
        !text ||
        `${item.name ?? app.display_name} ${app.description ?? ''}`
          .toLocaleLowerCase()
          .includes(text)
      );
    })
    .sort((a, b) =>
      sort === 'name'
        ? (workspace.items[a.miniapp_id]?.name ?? a.display_name).localeCompare(
            workspace.items[b.miniapp_id]?.name ?? b.display_name,
          )
        : sort === 'updated'
          ? b.updated_at_ms - a.updated_at_ms
          : (workspace.items[b.miniapp_id]?.last_opened ?? 0) -
              (workspace.items[a.miniapp_id]?.last_opened ?? 0) ||
            b.updated_at_ms - a.updated_at_ms,
    );
}
