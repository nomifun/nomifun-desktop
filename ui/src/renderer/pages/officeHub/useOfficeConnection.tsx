import { ipcBridge } from '@/common';
import { officeHub } from '@/common/adapter/officeHubBridge';
import type { AuthStartRequest } from '@/common/types/officeHub';
import { useCallback, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { OfficeErrorPanel, type AuthorizationProps } from './dialogs';
import { relatedAuthorization, terminalAuth } from './model';
import { useOfficeHub } from './useOfficeHub';

/**
 * Shared Feishu connection controller used by the connector management surface
 * and the Feishu knowledge page. Owns status, authorization flows, the busy
 * runner and error feedback; hosts only decide which dialogs to render.
 */
export function useOfficeConnection({ onNeedSettings }: { onNeedSettings?: () => void } = {}) {
  const { t } = useTranslation();
  const hub = useOfficeHub();
  const [busy, setBusy] = useState(false);
  const [actionError, setActionError] = useState<unknown>(null);
  const [authVisible, setAuthVisible] = useState(false);

  const run = useCallback(async (action: () => Promise<void>) => {
    if (busy) return;
    setBusy(true); setActionError(null);
    try { await action(); } catch (error) { setActionError(error); }
    finally { setBusy(false); }
  }, [busy]);

  const openExternal = useCallback(async (url: string): Promise<boolean> => {
    try {
      if (new URL(url).protocol !== 'https:') throw new Error(t('office.error'));
      await ipcBridge.shell.openExternal.invoke(url);
      return true;
    } catch (error) { setActionError(error); return false; }
  }, [t]);

  const startAuth = useCallback(async (request: AuthStartRequest) => {
    if (hub.flow && !terminalAuth(hub.flow.phase)) return;
    await run(async () => {
      try { hub.setFlow(await officeHub.startAuth(request)); }
      catch (error) { const flow = relatedAuthorization(error); if (!flow) throw error; hub.setFlow(await officeHub.auth(flow)); }
      await hub.refreshStatus();
    });
  }, [hub, run]);

  /** Returns false when connection settings must be opened first. */
  const authorize = useCallback((scopes: string[]): boolean => {
    if (!hub.status?.connection.profile || hub.status.runtime.phase !== 'ready') {
      onNeedSettings?.();
      return false;
    }
    setAuthVisible(true); setActionError(null);
    void startAuth({ kind: 'login', scopes: [...new Set(scopes)], profile: hub.status.connection.profile, app_id: null, app_secret: null });
    return true;
  }, [hub.status, startAuth, onNeedSettings]);

  const cancelAuth = useCallback(() => run(async () => {
    if (!hub.flow) return;
    const next = await officeHub.cancelAuth(hub.flow.id); hub.setFlow(next);
    if (next.phase === 'complete' || next.phase === 'partial') await hub.refreshStatus();
  }), [hub.flow, run, hub.refreshStatus]);

  const closeAuth = useCallback(() => {
    if (hub.flow?.phase === 'complete' || hub.flow?.phase === 'partial') void hub.refreshStatus();
    setAuthVisible(false);
  }, [hub.flow, hub.refreshStatus]);

  const resumeFlow = useCallback((id: string) => {
    void run(async () => { hub.setFlow(await officeHub.auth(id)); setAuthVisible(true); });
  }, [run, hub]);

  /** Clears local flow/operation artifacts after the account signs out. */
  const signedOut = useCallback(() => {
    hub.setFlow(null); hub.setOperation(null); setActionError(null);
  }, [hub]);

  const errorPanel = useCallback((error: unknown, showOperation?: (id: string) => void) => (
    <OfficeErrorPanel
      error={error}
      onShowOperation={showOperation}
      onResumeFlow={resumeFlow}
      onAuthorize={(scopes) => void authorize(scopes)}
      onOpenUrl={(url) => void openExternal(url)}
    />
  ), [resumeFlow, authorize, openExternal]);

  const authorization = {
    flow: hub.flow,
    busy,
    error: <>{errorPanel(actionError)}{errorPanel(hub.flowError)}</>,
    onCancel: () => void cancelAuth(),
    onOpen: (url: string) => void openExternal(url),
  } satisfies AuthorizationProps;

  return {
    hub, busy, actionError, setActionError, authVisible, setAuthVisible,
    run, openExternal, startAuth, authorize, cancelAuth, closeAuth, resumeFlow,
    signedOut, errorPanel, authorization,
  };
}

export type OfficeConnectionController = ReturnType<typeof useOfficeConnection>;
