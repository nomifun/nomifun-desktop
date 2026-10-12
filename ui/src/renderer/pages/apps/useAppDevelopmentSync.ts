import { ipcBridge } from '@/common';
import { appHosting } from '@/common/adapter/appHostingBridge';
import type { IConversationTurnCompletedEvent, IConversationTurnStartedEvent } from '@/common/adapter/ipcBridge';
import type { HostedApp } from '@/common/types/appHosting';
import { useEffect, useRef, useState } from 'react';
import { appErrorMessage, type AppPreviewFileChange } from './appHostingModel';

/** App chrome follows its ordinary Session and the command-hosting owner. */
export function useAppDevelopmentSync(app: HostedApp | undefined, refresh: () => Promise<unknown>, accept: (app: HostedApp) => Promise<unknown>, selectedSessionId?: string | null) {
  const [configuring, setConfiguring] = useState(false);
  const [configurationError, setConfigurationError] = useState('');
  const [previewFileChange, setPreviewFileChange] = useState<AppPreviewFileChange>();
  const latestApp = useRef(app);
  latestApp.current = app;
  const appId = app?.id;
  const sessionId = selectedSessionId === undefined ? app?.development_session_id : selectedSessionId;
  useEffect(() => {
    setConfiguring(false); setConfigurationError(''); setPreviewFileChange(undefined);
    if (!appId || !sessionId) return;
    let disposed = false;
    let lastRequestedTurn: string | undefined;
    let request: object | undefined;
    let turnBaseline: { turnId: string; pid: number | null; url: string | null | undefined } | undefined;
    const readOwner = () => { void refresh().catch(() => {}); };
    const refreshEvent = (event: { conversation_id: unknown }) => {
      if (String(event.conversation_id) === sessionId) readOwner();
    };
    const started = (event: IConversationTurnStartedEvent) => {
      if (String(event.conversation_id) !== sessionId) return;
      const current = latestApp.current;
      turnBaseline = event.turn_id && current?.id === appId ? {
        turnId: String(event.turn_id), pid: current.pid, url: current.settings.web_url,
      } : undefined;
      readOwner();
    };
    const completed = (event: IConversationTurnCompletedEvent) => {
      if (String(event.conversation_id) !== sessionId) return;
      readOwner();
      // The wire notification only addresses this request. The Host checks the
      // exact canonical completed Turn and current admission fence before any
      // configuration write. Mounting, polling, failures and pauses never do it.
      if (!event.turn_id || event.state !== 'ai_waiting_input' || event.runtime.is_processing !== false
        || event.runtime.can_send_message !== true || lastRequestedTurn === String(event.turn_id)) return;
      lastRequestedTurn = String(event.turn_id);
      const baseline = turnBaseline?.turnId === String(event.turn_id) ? turnBaseline : undefined;
      const currentRequest = {};
      request = currentRequest;
      setConfiguring(true); setConfigurationError('');
      void appHosting.update.invoke({ id: appId, detect_settings: true,
        completion: { session_id: sessionId, turn_id: String(event.turn_id) },
      }).then(async value => {
        if (disposed || request !== currentRequest) return;
        const { completion_observation: observation, ...owner } = value;
        if (owner.id !== appId || observation
          && (observation.session_id !== sessionId || observation.turn_id !== String(event.turn_id)
            || typeof observation.files_changed !== 'boolean')) throw new Error('Application configuration identity changed');
        await accept(owner);
        if (!disposed && request === currentRequest && observation?.files_changed === true && baseline
          && baseline.pid === owner.pid && baseline.url === owner.settings.web_url) {
          setPreviewFileChange({ appId, sessionId, turnId: String(event.turn_id), pid: baseline.pid, url: baseline.url });
        }
      }).catch(error => {
        if (!disposed && request === currentRequest) setConfigurationError(appErrorMessage(error));
      }).finally(() => {
        if (!disposed && request === currentRequest) { setConfiguring(false); readOwner(); }
      });
    };
    const dispose = [
      ipcBridge.conversation.turnStarted.on(started),
      ipcBridge.conversation.turnCompleted.on(completed),
      ipcBridge.conversation.turnPaused.on(refreshEvent),
      ipcBridge.conversation.listChanged.on(refreshEvent),
    ];
    return () => { disposed = true; dispose.forEach(stop => stop()); };
  }, [appId, sessionId, refresh, accept]);
  return { configuring, configurationError, previewFileChange };
}
