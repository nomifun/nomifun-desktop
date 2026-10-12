import { appHosting } from '@/common/adapter/appHostingBridge';
import type { HostedApp } from '@/common/types/appHosting';
import type { TChatConversation } from '@/common/config/storage';
import { Alert, Spin } from '@arco-design/web-react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useLocation, useNavigate, useParams } from 'react-router-dom';
import useSWR from 'swr';
import AppWorkspace from './AppWorkspace';
import AppDevelopmentConversation, { matchesAppDevelopmentConversation, type AppEditorMode } from './AppDevelopmentConversation';
import { appErrorMessage, isAppProjectSessionBusy } from './appHostingModel';
import { isConversationProcessing } from '@/renderer/pages/conversation/utils/conversationRuntime';
import AppWorkspaceFiles from './AppWorkspaceFiles';
import { moveGuidInputDraft } from '@/renderer/pages/guid/hooks/useGuidDraftState';
import { useAppDevelopmentSync } from './useAppDevelopmentSync';
import { useAppProjectSessions } from './useAppProjectSessions';
import AppProjectHistory from './AppProjectHistory';
import { addProjectWorkpath } from '@/renderer/pages/conversation/SessionList/utils/projectWorkpaths';
import { tryParseEntityId } from '@/common/types/ids';

export default function AppWorkspaceRoute() {
  const { id } = useParams();
  const location = useLocation();
  const navigate = useNavigate();
  const { t } = useTranslation();
  const isNew = location.pathname === '/apps/new';
  const isImport = location.pathname === '/apps/import';
  const intent = new URLSearchParams(location.search).get('intent');
  const mode: AppEditorMode = isImport || intent === 'import' ? 'import' : isNew || intent === 'create' ? 'create' : 'edit';
  const routeIdentity = useRef(location.pathname);
  routeIdentity.current = location.pathname;
  const locationIdentity = `${location.key}:${location.pathname}:${location.search}`;
  const selectionLocation = useRef(locationIdentity);
  const selectionGeneration = useRef(0);
  if (selectionLocation.current !== locationIdentity) {
    selectionLocation.current = locationIdentity;
    selectionGeneration.current += 1;
  }
  useEffect(() => () => { selectionGeneration.current += 1; }, []);
  const [conversation, setConversation] = useState<TChatConversation>();
  const [submitting, setSubmitting] = useState(false);
  const [manualBusy, setManualBusy] = useState(false);
  const [selectionError, setSelectionError] = useState('');
  const [selecting, setSelecting] = useState(false);
  useEffect(() => { setManualBusy(false); setSubmitting(false); }, [location.pathname]);
  const onSubmittingChange = useCallback((busy: boolean) => {
    if (routeIdentity.current === location.pathname) setSubmitting(busy);
  }, [location.pathname]);
  const onFileOperationChange = useCallback((busy: boolean) => {
    if (routeIdentity.current === location.pathname) setManualBusy(busy);
  }, [location.pathname]);
  const { data: app, error, isLoading, mutate } = useSWR(
    isNew ? `apps.new/${location.key}` : id ? `app/${id}` : null,
    () => isNew ? appHosting.create.invoke({}) : appHosting.get.invoke({ id: id! }),
    { refreshInterval: isNew ? 0 : 2000, revalidateOnFocus: !isNew, revalidateOnReconnect: !isNew, shouldRetryOnError: false },
  );
  const refreshOwner = useCallback(() => mutate(), [mutate]);
  const acceptConfiguration = useCallback((value: HostedApp) => mutate(value, { revalidate: false }), [mutate]);
  const defaultSelection = useRef<{ appId: string; sessionId: string | null } | undefined>(undefined);
  if (app && defaultSelection.current?.appId !== app.id) defaultSelection.current = { appId: app.id, sessionId: app.development_session_id };
  const requestedSession = new URLSearchParams(location.search).get('session');
  const invalidSession = requestedSession !== null && requestedSession !== 'new' && !tryParseEntityId('conversation', requestedSession);
  const selectedSessionId = requestedSession === 'new' ? null : requestedSession ?? defaultSelection.current?.sessionId;
  const project = useAppProjectSessions(app?.id);
  const { configuring, configurationError, previewFileChange } = useAppDevelopmentSync(app, refreshOwner, acceptConfiguration, selectedSessionId);
  useEffect(() => { if (app) addProjectWorkpath(app.directory); }, [app?.id, app?.directory]);
  useEffect(() => { setSubmitting(false); setSelectionError(''); setSelecting(false); }, [locationIdentity, id, selectedSessionId]);
  const openSessionRoute = useCallback((sessionId: string) => {
    if (!app) return;
    const search = new URLSearchParams(location.search); search.set('session', sessionId);
    void navigate(`/apps/${app.id}?${search}`);
  }, [app?.id, location.search, navigate]);
  const selectSession = useCallback(async (sessionId: string) => {
    if (!app) return;
    const generation = ++selectionGeneration.current;
    const requestedLocation = selectionLocation.current;
    const isCurrent = () => selectionGeneration.current === generation && selectionLocation.current === requestedLocation;
    setSelecting(true); setSelectionError('');
    try {
      const opened = await appHosting.developmentSession.invoke({ id: app.id, session_id: sessionId });
      if (!isCurrent()) return;
      if (opened.app.id !== app.id || !matchesAppDevelopmentConversation(opened.app, opened.conversation, sessionId)) throw new Error(t('appHosting.workspace.sessionMismatch'));
      await mutate(opened.app, { revalidate: false });
      if (isCurrent()) openSessionRoute(sessionId);
    } catch (error) { if (isCurrent()) setSelectionError(appErrorMessage(error)); }
    finally { if (isCurrent()) setSelecting(false); }
  }, [app, mutate, openSessionRoute, t]);
  useEffect(() => {
    if (isNew && app) void navigate(`/apps/${app.id}?intent=create`, { replace: true });
  }, [isNew, app, navigate]);
  const onAppChanged = useCallback((value: HostedApp) => {
    if (routeIdentity.current !== location.pathname || id && value.id !== id) return;
    if (!id) {
      if (mode === 'import' && !value.development_session_id) moveGuidInputDraft('app-workspace:import', `app-workspace:${value.id}:new`);
      void navigate(`/apps/${value.id}?intent=${mode}`, { replace: true });
    } else void mutate(value, { revalidate: true });
  }, [id, location.pathname, mode, navigate, mutate]);
  if (error && !app) return <Alert type='error' content={isNew ? t('appHosting.workspace.createFailed') : appErrorMessage(error)} />;
  if ((isLoading && !app) || isNew) return <Spin tip={t('appHosting.workspace.creating')} />;
  const currentConversation = app && conversation && matchesAppDevelopmentConversation(app, conversation, selectedSessionId) ? conversation : undefined;
  const projectBusy = project.items.some(item => isAppProjectSessionBusy(item.conversation));
  const siblingBusy = project.items.some(item => String(item.conversation.id) !== selectedSessionId && isAppProjectSessionBusy(item.conversation));
  const developmentBusy = submitting || projectBusy || Boolean(currentConversation && isConversationProcessing(currentConversation));
  const history = app ? <AppProjectHistory items={project.items} selectedSessionId={selectedSessionId} selectedName={currentConversation?.name}
    total={project.total} loading={project.isLoading || project.isValidating || selecting} error={selectionError || (project.error ? appErrorMessage(project.error) : undefined)} hasMore={project.hasMore}
    newSessionDisabled={manualBusy || configuring || submitting || projectBusy}
    onSelect={sessionId => { void selectSession(sessionId); }} onOpenConversation={sessionId => { void navigate(`/conversation/${sessionId}`); }}
    onNewSession={() => {
      selectionGeneration.current += 1; setSelecting(false); setSelectionError(''); openSessionRoute('new');
    }} onLoadMore={project.loadMore} onRefresh={() => { void project.refresh().catch(() => {}); }} /> : undefined;
  return <AppWorkspace key={id ?? mode} app={app} mode={mode}
    configurationBusy={configuring} configurationError={error ? appErrorMessage(error) : configurationError} previewFileChange={previewFileChange}
    developmentBusy={developmentBusy} projectHistory={history}
    onFileOperationChange={onFileOperationChange}
    conversation={invalidSession ? <Alert type='error' content={t('appHosting.workspace.sessionMismatch')} /> : <AppDevelopmentConversation key={selectedSessionId ?? 'new'} app={app} mode={mode} selectedSessionId={selectedSessionId} onSessionCreated={sessionId => {
      openSessionRoute(sessionId); void project.refresh().catch(() => {});
    }} onAppChanged={onAppChanged} onConversationLoaded={setConversation} onSubmittingChange={onSubmittingChange} submissionDisabled={manualBusy || configuring || siblingBusy} />}
    filePanel={currentConversation ? <AppWorkspaceFiles conversation={currentConversation} /> : undefined}
    onAppChanged={onAppChanged} onRefresh={() => { void refreshOwner().catch(() => {}); }}
    onDeleted={() => void navigate('/apps', { replace: true })} />;
}
