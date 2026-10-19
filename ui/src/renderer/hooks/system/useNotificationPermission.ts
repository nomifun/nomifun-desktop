import { ipcBridge } from '@/common';
import type { SystemPermissionState } from '@/common/adapter/ipcBridge';
import { isDesktopShell } from '@/renderer/utils/platform';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useSystemPermissions } from './useSystemPermissions';

type NotificationSnapshot = {
  state: SystemPermissionState;
  error: boolean;
  loading: boolean;
  busy: boolean;
};

const permissionState = (state: string): SystemPermissionState =>
  state === 'granted' ? 'granted' : state === 'denied' ? 'denied' : state === 'default' ? 'not_determined' : 'unknown';

/** One live notification grant shared by the status row and preference controls. */
export const useNotificationPermission = () => {
  const desktopShell = isDesktopShell();
  const system = useSystemPermissions(desktopShell);
  const [snapshot, setSnapshot] = useState<NotificationSnapshot>({
    state: 'unknown', error: false, loading: desktopShell, busy: false,
  });
  const revision = useRef(0);
  const mounted = useRef(true);
  const activeRead = useRef<Promise<SystemPermissionState | null> | null>(null);

  const readPermission = useCallback((request: boolean): Promise<SystemPermissionState | null> => {
    if (!desktopShell || !mounted.current) return Promise.resolve(null);
    const currentRevision = ++revision.current;
    setSnapshot(previous => ({ ...previous, loading: !request, busy: request }));
    const pending = (async () => {
      try {
        const result = await (request
          ? ipcBridge.notification.requestPermission.invoke()
          : ipcBridge.notification.permissionState.invoke());
        if (!mounted.current || currentRevision !== revision.current) return null;
        const state = permissionState(result);
        setSnapshot({ state, error: false, loading: false, busy: false });
        return state;
      } catch {
        if (!mounted.current || currentRevision !== revision.current) return null;
        setSnapshot({ state: 'unknown', error: true, loading: false, busy: false });
        return null;
      }
    })();
    if (currentRevision === revision.current) activeRead.current = pending;
    return pending;
  }, [desktopShell]);

  // Dismissing an OS prompt can trigger a newer focus check before the original
  // request settles. Preference writes must use that latest completed check.
  const settledPermission = useCallback(async (): Promise<SystemPermissionState | null> => {
    while (mounted.current) {
      const pending = activeRead.current;
      if (!pending) return null;
      const state = await pending;
      if (pending === activeRead.current) return mounted.current ? state : null;
    }
    return null;
  }, []);

  useEffect(() => {
    mounted.current = true;
    void readPermission(false);
    const onFocus = () => void readPermission(false);
    window.addEventListener('focus', onFocus);
    return () => {
      mounted.current = false;
      revision.current += 1;
      window.removeEventListener('focus', onFocus);
    };
  }, [readPermission]);

  const refreshSystem = system.refresh;
  const refresh = useCallback(async () => {
    const [state] = await Promise.all([readPermission(false), refreshSystem()]);
    return state;
  }, [readPermission, refreshSystem]);
  const request = useCallback(() => readPermission(true), [readPermission]);
  const ensureGranted = useCallback(async (): Promise<boolean> => {
    // The browser has no native notification capability. Keep the existing
    // preference behavior there without invoking a host authorization prompt.
    if (!desktopShell) return true;
    await readPermission(false);
    const state = await settledPermission();
    if (state === 'granted') return true;
    if (state === null) return false;
    await readPermission(true);
    return await settledPermission() === 'granted';
  }, [desktopShell, readPermission, settledPermission]);
  const openSystemSettings = system.openSettings;
  const openSettings = useCallback(() => openSystemSettings('notifications'), [openSystemSettings]);

  return {
    ...snapshot,
    error: snapshot.error || Boolean(system.error),
    desktopShell,
    canOpenSettings: desktopShell && system.status?.platform === 'macos',
    refresh, request, ensureGranted, openSettings,
  };
};

export type NotificationPermission = ReturnType<typeof useNotificationPermission>;
