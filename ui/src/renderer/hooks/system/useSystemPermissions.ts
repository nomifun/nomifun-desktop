import { ipcBridge } from '@/common';
import type {
  SystemPermissionKind,
  SystemPermissionStatus,
} from '@/common/adapter/ipcBridge';
import { useCallback, useEffect, useState } from 'react';

export const useSystemPermissions = (enabled = true) => {
  const [status, setStatus] = useState<SystemPermissionStatus | null>(null);
  const [loading, setLoading] = useState(enabled);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    if (!enabled) return null;
    setLoading(true);
    try {
      const next = await ipcBridge.systemPermissions.get.invoke();
      setStatus(next);
      setError(null);
      return next;
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught));
      return null;
    } finally {
      setLoading(false);
    }
  }, [enabled]);

  useEffect(() => {
    if (!enabled) {
      setLoading(false);
      return undefined;
    }
    void refresh();
    const onFocus = () => void refresh();
    window.addEventListener('focus', onFocus);
    return () => window.removeEventListener('focus', onFocus);
  }, [enabled, refresh]);

  const request = useCallback(async (kind: SystemPermissionKind) => {
    setLoading(true);
    try {
      const next = await ipcBridge.systemPermissions.request.invoke({ kind });
      setStatus(next);
      setError(null);
      return next;
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught));
      return null;
    } finally {
      setLoading(false);
    }
  }, []);

  const openSettings = useCallback(async (kind: SystemPermissionKind) => {
    try {
      await ipcBridge.systemPermissions.openSettings.invoke({ kind });
      setError(null);
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught));
    }
  }, []);

  return { error, loading, openSettings, refresh, request, status };
};
