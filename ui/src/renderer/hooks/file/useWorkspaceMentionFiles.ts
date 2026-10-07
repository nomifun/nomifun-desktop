import type { IWorkspaceFlatFile } from '@/common/adapter/ipcBridge';
import type { FileOrFolderItem } from '@/renderer/utils/file/fileTypes';
import { useCallback, useEffect, useState } from 'react';

type Listing = {
  key: string;
  attempt: number;
  status: 'loading' | 'ready' | 'error';
  items: FileOrFolderItem[];
};

const emptyItems: FileOrFolderItem[] = [];

export function useWorkspaceMentionFiles({ workspace, sessionKey, enabled, listFiles }: {
  workspace: string | undefined;
  sessionKey: string | null;
  enabled: boolean;
  listFiles: (root: string) => Promise<IWorkspaceFlatFile[]>;
}) {
  const key = enabled && workspace && sessionKey ? JSON.stringify([workspace, sessionKey]) : null;
  const [attempt, setAttempt] = useState(0);
  const [listing, setListing] = useState<Listing | null>(null);

  useEffect(() => {
    if (!key || !workspace) {
      setListing(null);
      return;
    }

    let cancelled = false;
    setListing({ key, attempt, status: 'loading', items: emptyItems });
    void (async () => {
      try {
        const files = await listFiles(workspace);
        if (cancelled) return;
        setListing({
          key, attempt, status: 'ready',
          items: files.map((item) => ({
            path: item.fullPath,
            name: item.name,
            isFile: true,
            relativePath: item.relativePath || undefined,
          })),
        });
      } catch (error) {
        if (cancelled) return;
        console.warn('[SendBox] Failed to load workspace file mentions:', error);
        setListing({ key, attempt, status: 'error', items: emptyItems });
      }
    })();
    return () => { cancelled = true; };
  }, [attempt, key, listFiles, workspace]);

  // Hide an old source/attempt synchronously, before passive effects start its
  // replacement request. Keyboard selection sees the same current items.
  const current = listing?.key === key && listing.attempt === attempt ? listing : null;
  const retry = useCallback(() => {
    if (key) setAttempt((value) => value + 1);
  }, [key]);

  return {
    items: key && current?.status === 'ready' ? current.items : emptyItems,
    loading: Boolean(key) && (!current || current.status === 'loading'),
    hasError: Boolean(key) && current?.status === 'error',
    retry,
  };
}
