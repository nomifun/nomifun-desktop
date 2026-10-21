import { ipcBridge } from '@/common';
import type {
  SystemPermissionKind,
  SystemPermissionStatus,
} from '@/common/adapter/ipcBridge';
import { useCallback, useEffect, useRef, useState } from 'react';

type PermissionSnapshot = {
  scope: string;
  status: SystemPermissionStatus | null;
  loading: boolean;
  error: string | null;
};

/** Only the latest live check for the current consumer may update readiness. */
export const useSystemPermissions = (enabled = true, scope = '') => {
  const [snapshot, setSnapshot] = useState<PermissionSnapshot>({ scope, status: null, loading: enabled, error: null });
  const generation = useRef(0);
  const inFlightRequest = useRef<{ scope: string; generation: number } | null>(null);
  const mounted = useRef(true);
  const current = useRef({ enabled, scope });
  current.current = { enabled, scope };
  const isCurrent = useCallback((requestGeneration: number) => mounted.current && current.current.enabled &&
    current.current.scope === scope && generation.current === requestGeneration, [scope]);
  const hasCurrentRequest = useCallback(() => {
    const owner = inFlightRequest.current;
    return owner?.scope === scope && isCurrent(owner.generation);
  }, [isCurrent, scope]);
  const begin = useCallback(() => {
    const requestGeneration = ++generation.current;
    setSnapshot(previous => ({ scope, status: previous.scope === scope ? previous.status : null,
      error: previous.scope === scope ? previous.error : null, loading: true }));
    return requestGeneration;
  }, [scope]);

  const refresh = useCallback(async () => {
    // A TCC prompt can refocus the window before its POST settles. A GET must
    // not invalidate that result or unlock another explicit native request.
    if (!enabled || !mounted.current || !current.current.enabled || current.current.scope !== scope || hasCurrentRequest()) return null;
    const requestGeneration = begin();
    try {
      const next = await ipcBridge.systemPermissions.get.invoke();
      if (!isCurrent(requestGeneration)) return null;
      setSnapshot({ scope, status: next, error: null, loading: false });
      return next;
    } catch (caught) {
      if (isCurrent(requestGeneration)) setSnapshot(previous => ({ ...previous, loading: false,
        error: caught instanceof Error ? caught.message : String(caught) }));
      return null;
    }
  }, [begin, enabled, hasCurrentRequest, isCurrent, scope]);

  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; generation.current++; inFlightRequest.current = null; };
  }, []);

  useEffect(() => {
    if (!enabled) {
      generation.current++;
      return undefined;
    }
    void refresh();
    const onFocus = () => void refresh();
    window.addEventListener('focus', onFocus);
    return () => {
      generation.current++;
      window.removeEventListener('focus', onFocus);
    };
  }, [enabled, refresh]);

  const request = useCallback(async (kind: SystemPermissionKind, onError?: () => void) => {
    if (!enabled || !mounted.current || !current.current.enabled || current.current.scope !== scope || hasCurrentRequest()) return null;
    const requestGeneration = begin();
    const owner = { scope, generation: requestGeneration };
    // Set this before invoking IPC, including before React commits loading.
    inFlightRequest.current = owner;
    try {
      const next = await ipcBridge.systemPermissions.request.invoke({ kind });
      if (!isCurrent(requestGeneration)) return null;
      setSnapshot({ scope, status: next, error: null, loading: false });
      return next;
    } catch (caught) {
      if (isCurrent(requestGeneration)) {
        setSnapshot(previous => ({ ...previous, loading: false,
          error: caught instanceof Error ? caught.message : String(caught) }));
        onError?.();
      }
      return null;
    } finally {
      // An old scope's response cannot release a newer scope's request.
      if (inFlightRequest.current === owner) inFlightRequest.current = null;
    }
  }, [begin, enabled, hasCurrentRequest, isCurrent, scope]);

  const openSettings = useCallback(async (kind: SystemPermissionKind) => {
    if (!enabled || !mounted.current || !current.current.enabled || current.current.scope !== scope || hasCurrentRequest()) return;
    const requestGeneration = ++generation.current;
    setSnapshot(previous => ({ ...previous, scope, loading: false, error: null }));
    try {
      await ipcBridge.systemPermissions.openSettings.invoke({ kind });
    } catch (caught) {
      if (isCurrent(requestGeneration)) setSnapshot(previous => ({ ...previous,
        error: caught instanceof Error ? caught.message : String(caught) }));
    }
  }, [enabled, hasCurrentRequest, isCurrent, scope]);

  const currentSnapshot = snapshot.scope === scope ? snapshot : { scope, status: null, loading: enabled, error: null };
  return { error: currentSnapshot.error, loading: enabled && currentSnapshot.loading,
    openSettings, refresh, request, status: currentSnapshot.status };
};
