import { ipcBridge } from '@/common';
import { appHosting } from '@/common/adapter/appHostingBridge';
import type { TChatConversation } from '@/common/config/storage';
import type { HostedApp } from '@/common/types/appHosting';
import { Alert, Spin } from '@arco-design/web-react';
import { useCallback, useEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import useSWR from 'swr';
import GuidPage, { type GuidProductEntry } from '@/renderer/pages/guid/GuidPage';
import { GuidDraftScopeContext } from '@/renderer/pages/guid/hooks/useGuidDraftState';
import ChatConversation from '@/renderer/pages/conversation/components/ChatConversation';
import { officialConversationTemplateKey } from '@/renderer/pages/conversation/components/conversationAgentIdentity';
import { getConversationOrNull } from '@/renderer/pages/conversation/utils/conversationCache';
import { parseConversationId } from '@/common/types/ids';
import { uuidv7 } from '@/common/utils';
import { useGuidDraftState } from '@/renderer/pages/guid/hooks/useGuidDraftState';
import { appErrorMessage } from './appHostingModel';

export type AppEditorMode = 'create' | 'import' | 'edit';
type Props = {
  app?: HostedApp;
  mode: AppEditorMode;
  onAppChanged: (app: HostedApp) => void;
  onConversationLoaded?: (conversation: TChatConversation | undefined) => void;
  onSubmittingChange?: (submitting: boolean) => void;
  submissionDisabled?: boolean;
  /** null is a new draft; undefined follows the initial navigation reference. */
  selectedSessionId?: string | null;
  onSessionCreated?: (id: string) => void;
};

export function matchesAppDevelopmentConversation(app: HostedApp, conversation: TChatConversation, sessionId: string | null | undefined = app.development_session_id): boolean {
  const path = (value: string) => value.replace(/\\/g, '/').replace(/\/$/, '');
  return conversation.type === 'nomi'
    && String(conversation.id) === sessionId
    && conversation.session_purpose !== 'plugin_authoring'
    && officialConversationTemplateKey(conversation.extra) === 'app.developer'
    && path(conversation.extra.workspace) === path(app.directory);
}

/** The Apps surface hosts the existing conversation and composer, not a new transcript. */
export default function AppDevelopmentConversation(props: Props) {
  const scope = `app-workspace:${props.app?.id ?? props.mode}${props.selectedSessionId === null ? ':new' : ''}`;
  return <GuidDraftScopeContext.Provider value={scope}><AppDevelopmentConversationBody key={scope} {...props} /></GuidDraftScopeContext.Provider>;
}

function AppDevelopmentConversationBody({ app, mode, onAppChanged, onConversationLoaded, onSubmittingChange, submissionDisabled, selectedSessionId, onSessionCreated }: Props) {
  const { t } = useTranslation();
  const sessionId = selectedSessionId === undefined ? app?.development_session_id : selectedSessionId;
  const [creationKey, setCreationKey] = useGuidDraftState('session-creation-key', uuidv7);
  const pendingApp = useRef<HostedApp | undefined>(undefined);
  const { data: conversation, error, isLoading, mutate } = useSWR(
    sessionId ? `conversation/${sessionId}` : null,
    () => getConversationOrNull(parseConversationId(sessionId!)),
    { refreshInterval: 2000 },
  );
  useEffect(() => {
    if (!sessionId) return;
    const refresh = (event: { conversation_id: unknown }) => {
      if (String(event.conversation_id) === sessionId) void mutate();
    };
    const dispose = [
      ipcBridge.conversation.listChanged.on(refresh),
      ipcBridge.conversation.turnStarted.on(refresh),
      ipcBridge.conversation.turnCompleted.on(refresh),
      ipcBridge.conversation.turnPaused.on(refresh),
    ];
    return () => dispose.forEach(stop => stop());
  }, [sessionId, mutate]);
  const matching = app && conversation ? matchesAppDevelopmentConversation(app, conversation, sessionId) : false;
  useEffect(() => {
    onConversationLoaded?.(matching ? conversation ?? undefined : undefined);
  }, [matching, conversation, onConversationLoaded]);
  const createConversation: GuidProductEntry['createConversation'] = useCallback(async request => {
    const current = app ?? pendingApp.current ?? await appHosting.create.invoke({});
    pendingApp.current = current;
    // Persist before the request: an unavailable response must not mint a
    // second Session when this same unsent project draft is reopened.
    if (selectedSessionId === null) setCreationKey(creationKey);
    const opened = await appHosting.developmentSession.invoke({
      id: current.id, model: request.model, reasoning_effort: request.reasoning_effort,
      refresh_workflow: true,
      ...(selectedSessionId === null ? { new_session: true, creation_key: creationKey } : {}),
    });
    if (!matchesAppDevelopmentConversation(opened.app, opened.conversation, String(opened.conversation.id))) {
      throw new Error(t('appHosting.workspace.sessionMismatch'));
    }
    pendingApp.current = opened.app;
    return opened.conversation;
  }, [app, selectedSessionId, creationKey, setCreationKey, t]);
  const onCreated = useCallback((created: TChatConversation) => {
    if (pendingApp.current) onAppChanged(pendingApp.current);
    setCreationKey(uuidv7());
    onSessionCreated?.(String(created.id));
  }, [onAppChanged, onSessionCreated, setCreationKey]);
  const prepareSubmission = useCallback(async () => {
    if (!app || !sessionId) throw new Error(t('appHosting.workspace.sessionMismatch'));
    onSubmittingChange?.(true);
    try {
      const opened = await appHosting.developmentSession.invoke({ id: app.id, ...(selectedSessionId === undefined ? {} : { session_id: sessionId }), refresh_workflow: true });
      if (opened.app.id !== app.id || String(opened.conversation.id) !== sessionId
        || !matchesAppDevelopmentConversation(opened.app, opened.conversation, sessionId)) throw new Error(t('appHosting.workspace.sessionMismatch'));
      onAppChanged(opened.app);
      await mutate(opened.conversation, { revalidate: false });
    } finally { onSubmittingChange?.(false); }
  }, [app, sessionId, selectedSessionId, onSubmittingChange, onAppChanged, mutate, t]);
  if (error && !conversation) return <Alert type='error' content={appErrorMessage(error)} />;
  if (sessionId && isLoading) return <Spin />;
  if (sessionId && conversation === null) return <Alert type='error' content={t('appHosting.workspace.sessionMismatch')} />;
  if (conversation && !matching) return <Alert type='error' content={t('appHosting.workspace.sessionMismatch')} />;
  if (matching && conversation) return <>
    {error && <Alert type='warning' content={appErrorMessage(error)} />}
    <ChatConversation key={conversation.id} conversation={conversation} embedded fixedAgentLabel={t('appHosting.workspace.fixedAgent')} submissionDisabled={submissionDisabled} onBeforeSubmit={prepareSubmission} />
  </>;
  return <GuidPage productEntry={{
      templateKey: 'app.developer', agentLabel: t('appHosting.workspace.fixedAgent'),
      workspace: app?.directory,
      placeholder: t(mode === 'import' ? 'appHosting.workspace.importPlaceholder' : mode === 'edit' ? 'appHosting.workspace.editPlaceholder' : 'appHosting.workspace.createPlaceholder'),
      createConversation, onConversationCreated: onCreated,
      onSubmittingChange,
      submissionDisabled,
    }} />;
}
