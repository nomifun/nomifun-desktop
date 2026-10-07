import { agentPlatform, conversation } from '@/common/adapter/ipcBridge';
import { pluginPlatform } from '@/common/adapter/pluginPlatformBridge';
import { configService } from '@/common/config/configService';
import type { IProvider, TChatConversation, TProviderWithModel } from '@/common/config/storage';
import type { NativeAgentExecution } from '@/common/types/agentPlatform';
import type { PluginDevelopmentPreflight } from '@/common/types/pluginDevelopment';
import { parseConversationId } from '@/common/types/ids';
import { uuidv7 } from '@/common/utils';
import { capabilityOf, capabilitySupportsTechnicalCapability } from '@/common/utils/providerModels';
import { reasoningEffortsForProtocol, type SessionReasoningEffort } from '@/common/types/reasoningEffort';
import ChatModelSelector from '@/renderer/components/chat/ChatModelSelector';
import { useAgentPresets } from '@/renderer/hooks/agent/useAgentPresets';
import { useModelsForTask } from '@/renderer/hooks/agent/useModelsForTask';
import { filterConversationAgentPresets, isConversationAgentTemplate } from '@/renderer/components/agent/conversationAgentCatalog';
import NomiChat from '@/renderer/pages/conversation/platforms/nomi/NomiChat';
import { PreviewProvider } from '@/renderer/pages/conversation/Preview';
import { useNomiModelSelection } from '@/renderer/pages/conversation/platforms/nomi/useNomiModelSelection';
import GuidAgentSelector from '../guid/components/GuidAgentSelector';
import type { GuidAgentSelection } from '../guid/types';
import { isExecutableAgentPreset, readGuidDefaultAgentSelection } from '../guid/hooks/agentSelectionUtils';
import { officialConversationTemplateKey } from '../conversation/components/conversationAgentIdentity';
import { TEMPLATE_I18N_PATH } from '../agentSettings/model';
import PluginWorkspace from './PluginWorkspace';
import PluginAuthoringArtifacts from './PluginAuthoringArtifacts';
import { continuePluginConversation, isPluginConversationPause, replyToPluginConversation } from './pluginConversationResume';
import { Alert, Button, Input, Spin } from '@arco-design/web-react';
import { Code, Left, Magic, MessageOne, Notes, PauseOne, Send, Timer, FolderOpen, Lightning, Right } from '@icon-park/react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useLocation, useNavigate, useParams } from 'react-router-dom';
import { isDesktopShell } from '@/renderer/utils/platform';
import styles from './PluginAuthoringPage.module.css';

type PendingSubmission = { text: string; key: string; input?: string; pause?: NativeAgentExecution };

export default function PluginAuthoringPage() {
  const { t } = useTranslation();
  const { sessionId } = useParams();
  const location = useLocation();
  const navigate = useNavigate();
  const desktop = isDesktopShell();
  const search = new URLSearchParams(location.search);
  const draftId = search.get('draft_id') ?? undefined;
  const pluginId = search.get('plugin_id') ?? undefined;
  const revision = search.get('expected_plugin_revision');
  const template = search.get('template');
  const [selection, setSelection] = useState<GuidAgentSelection>(readGuidDefaultAgentSelection);
  const agents = useAgentPresets();
  const models = useModelsForTask('chat');
  const [model, setModel] = useState<TProviderWithModel>();
  const [reasoningEffort, setReasoningEffort] = useState<SessionReasoningEffort>();
  const [session, setSession] = useState<TChatConversation | null>(null);
  const [execution, setExecution] = useState<NativeAgentExecution | null>(null);
  const [preflight, setPreflight] = useState<PluginDevelopmentPreflight | null>(null);
  const [input, setInput] = useState('');
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(Boolean(sessionId));
  const [artifactsOpen, setArtifactsOpen] = useState(true);
  const inputRef = useRef<HTMLTextAreaElement | null>(null);
  const busyRef = useRef(false);
  const loadEpoch = useRef(0);
  const pendingSend = useRef<PendingSubmission | null>(null);
  const pendingCreate = useRef<{ signature: string; key: string } | null>(null);
  const presets = filterConversationAgentPresets(agents.presets, agents.library?.active_bindings ?? []).filter(isExecutableAgentPreset);
  const templates = (agents.library?.official_templates ?? []).filter(isConversationAgentTemplate);
  const frozenTemplate = session ? officialConversationTemplateKey(session.extra) : null;
  const boundAgentLabel = frozenTemplate
    ? t(`agentSettings.template.${TEMPLATE_I18N_PATH[frozenTemplate]}.name`)
    : session?.agent_snapshot?.preset_name ?? t('pluginPlatform.authoring.boundAgent');

  useEffect(() => {
    if (model || !models.groups.length) return;
    const stored = configService.get('nomi.defaultModel');
    const group = models.groups.find(item => item.provider.id === stored?.provider_id && item.models.includes(stored.model)) ?? models.groups[0];
    const name = group.models.includes(stored?.model ?? '') ? stored!.model : group.models[0];
    setModel({ ...group.provider, use_model: name });
  }, [models.groups, model]);

  const load = useCallback(async (id: string) => {
    const epoch = ++loadEpoch.current;
    await pluginPlatform.authoring.getSession.invoke({ agent_session_id: id });
    const value = await conversation.get.invoke({ conversation_id: parseConversationId(id) });
    if (!value || value.type !== 'nomi') throw new Error(t('pluginPlatform.authoring.workspaceUnavailable'));
    const current = await agentPlatform.sessions.getExecution.invoke({ agent_session_id: id });
    if (epoch !== loadEpoch.current) return;
    setSession(value); setModel(value.model); setReasoningEffort(value.reasoning_effort); setExecution(current);
  }, [t]);

  useEffect(() => {
    setSession(null); setExecution(null);
    if (!sessionId || !desktop) { setLoading(false); return; }
    let active = true;
    setLoading(true);
    void load(sessionId).catch(caught => { if (active) setError(String(caught instanceof Error ? caught.message : caught)); })
      .finally(() => { if (active) setLoading(false); });
    return () => { active = false; loadEpoch.current += 1; };
  }, [sessionId, desktop, load]);

  useEffect(() => {
    if (!sessionId || !desktop) return;
    const refresh = (event: { conversation_id: string }) => {
      if (event.conversation_id === sessionId) void load(sessionId).catch(caught => setError(String(caught instanceof Error ? caught.message : caught)));
    };
    const off = [conversation.turnStarted.on(refresh), conversation.turnPaused.on(refresh), conversation.turnCompleted.on(refresh),
      conversation.reconnected.on(() => { void load(sessionId).catch(() => undefined); })];
    return () => { off.forEach(unsubscribe => unsubscribe()); };
  }, [sessionId, desktop, load]);

  useEffect(() => {
    if (!desktop || sessionId) return;
    let active = true;
    setPreflight(null);
    void pluginPlatform.authoring.preflight.invoke({ selection })
      .then(value => { if (active) setPreflight(value); })
      .catch(caught => { if (active) setError(String(caught instanceof Error ? caught.message : caught)); });
    return () => { active = false; };
  }, [selection, desktop, sessionId]);

  const ensureSession = async () => {
    if (session) return session;
    if (sessionId) throw new Error(t('pluginPlatform.authoring.workspaceUnavailable'));
    if (!desktop || !model || preflight?.status !== 'ready') throw new Error(t('pluginPlatform.authoring.moduleNeeded'));
    const request = {
      selection, model: { provider_id: model.id, model: model.use_model },
      ...(pluginId ? { plugin_id: pluginId } : {}),
      ...(revision !== null ? { expected_plugin_revision: Number(revision) } : {}),
      ...(draftId ? { draft_id: draftId } : {}),
      ...(reasoningEffort !== undefined ? { reasoning_effort: reasoningEffort } : {}),
    };
    const signature = JSON.stringify(request);
    const operation = pendingCreate.current?.signature === signature ? pendingCreate.current : { signature, key: uuidv7() };
    pendingCreate.current = operation;
    const created = await pluginPlatform.authoring.createSession.invoke({ ...request, idempotency_key: operation.key });
    await pluginPlatform.authoring.getSession.invoke({ agent_session_id: created.agent_session_id });
    const value = await conversation.get.invoke({ conversation_id: parseConversationId(created.agent_session_id) });
    if (!value || value.type !== 'nomi') throw new Error(t('pluginPlatform.authoring.workspaceUnavailable'));
    setSession(value); setModel(value.model); setReasoningEffort(value.reasoning_effort);
    // Keep the canonical identity in the plugin URL before admitting any input.
    await navigate(`/plugins/authoring/${encodeURIComponent(value.id)}${location.search}`, { replace: true });
    return value;
  };

  const openDraft = async () => {
    if (busyRef.current || !draftId || !desktop) return;
    busyRef.current = true; setBusy(true); setError('');
    try { await ensureSession(); }
    catch (caught) { setError(String(caught instanceof Error ? caught.message : caught)); }
    finally { busyRef.current = false; setBusy(false); }
  };

  const send = async () => {
    const text = input.trim();
    if (!desktop || !text || busyRef.current || running) return;
    const operation: PendingSubmission = pendingSend.current?.text === text ? pendingSend.current : {
      text, key: uuidv7(),
      ...(execution && isPluginConversationPause(execution) ? { pause: execution } : {}),
    };
    pendingSend.current = operation;
    busyRef.current = true; setBusy(true); setError('');
    try {
      const current = await ensureSession();
      const prefix = !session && pluginId ? t('pluginPlatform.authoring.editRequest', { id: pluginId, revision: revision ?? '' })
        : !session && draftId ? t('pluginPlatform.authoring.resumeDraftRequest', { id: draftId })
          : !session && template === 'agent.before_tool' ? t('pluginPlatform.authoring.templatePrompt') : '';
      operation.input ??= prefix ? `${prefix}\n\n${text}` : text;
      const message = {
        conversation_id: current.id, input: operation.input,
        idempotency_key: operation.key, plugin_delivery: draftId ? { draft_id: draftId } : {},
      };
      if (operation.pause) await replyToPluginConversation(message, operation.pause);
      else await continuePluginConversation(message);
      pendingSend.current = null; setInput(''); await load(current.id);
    } catch (caught) { setError(String(caught instanceof Error ? caught.message : caught)); }
    finally { busyRef.current = false; setBusy(false); }
  };

  const running = execution?.state === 'running' || (execution?.state !== 'paused' && session?.runtime?.is_processing === true);
  const paused = execution?.state === 'paused';
  const modelSelection = useNomiModelSelection({
    initialModel: session?.model ?? model,
    onSelectModel: async (provider: IProvider, name: string) => {
      if (!desktop || busyRef.current || running || paused) return false;
      if (!session) {
        const capability = capabilityOf(provider, name, 'chat');
        const options = capabilitySupportsTechnicalCapability(capability, 'reasoning') ? reasoningEffortsForProtocol(capability?.protocol) : [];
        if (reasoningEffort !== undefined && !options.includes(reasoningEffort)) setReasoningEffort(undefined);
        setModel({ ...provider, use_model: name }); return true;
      }
      busyRef.current = true; setBusy(true); setError('');
      try {
        const changed = await conversation.switchModel.invoke({ conversation_id: session.id, provider_id: provider.id, model: name });
        if (!changed) return false;
        await load(session.id); return true;
      } catch (caught) { setError(String(caught instanceof Error ? caught.message : caught)); return false; }
      finally { busyRef.current = false; setBusy(false); }
    },
    readOnly: !desktop,
  });
  const modelCapability = capabilityOf(models.groups.find(group => group.provider.id === modelSelection.current_model?.id)?.provider,
    modelSelection.current_model?.use_model ?? '', 'chat');
  const reasoningOptions = capabilitySupportsTechnicalCapability(modelCapability, 'reasoning') ? reasoningEffortsForProtocol(modelCapability?.protocol) : [];
  const changeReasoning = async (next: SessionReasoningEffort | undefined) => {
    if (!desktop || busyRef.current || loading || running || paused || session?.runtime?.state === 'starting') return;
    if (next !== undefined && !reasoningOptions.includes(next)) return;
    if (!session) { setReasoningEffort(next); return; }
    busyRef.current = true; setBusy(true); setError('');
    try {
      const result = await agentPlatform.sessions.updateReasoning.invoke({ agent_session_id: session.id, reasoning_effort: next });
      setReasoningEffort(result.reasoning_effort); await load(session.id);
    } catch (caught) { setError(String(caught instanceof Error ? caught.message : caught)); }
    finally { busyRef.current = false; setBusy(false); }
  };

  const stop = async () => {
    if (!session || busyRef.current) return;
    busyRef.current = true; setBusy(true); setError('');
    try { await conversation.stop.invoke({ conversation_id: session.id }); await load(session.id); }
    catch (caught) { setError(String(caught instanceof Error ? caught.message : caught)); }
    finally { busyRef.current = false; setBusy(false); }
  };
  const runAuthoringOperation = async (operation: () => Promise<void>) => {
    if (!desktop || !session || busyRef.current) throw new Error(t('pluginPlatform.authoring.submissionBusy'));
    busyRef.current = true; setBusy(true);
    try { await operation(); await load(session.id); }
    finally { busyRef.current = false; setBusy(false); }
  };

  const ideas = [
    { key: 'timer' as const, icon: <Timer /> },
    { key: 'notes' as const, icon: <Notes /> },
    { key: 'organizer' as const, icon: <FolderOpen /> },
    { key: 'automation' as const, icon: <Lightning /> },
  ];
  const artifactEmpty = <div className={styles.artifactEmpty}>
    <span className={styles.emptyMark}><Code size={24} /></span>
    <h3>{t('pluginPlatform.authoring.artifactEmptyTitle')}</h3>
    <p>{t('pluginPlatform.authoring.artifactEmptyBody')}</p>
  </div>;

  return <PluginWorkspace activeView='drafts'>
    <div className={styles.page}>
      <header className={styles.header}>
        <button type='button' className={styles.back} aria-label={t('pluginPlatform.authoring.backToLibrary')}
          title={t('pluginPlatform.authoring.backToLibrary')} onClick={() => navigate('/plugins?view=drafts')}><Left /></button>
        <div className={styles.heading}><h1>{session?.name || t('pluginPlatform.authoring.title')}</h1>
          <p>{t('pluginPlatform.authoring.workspaceHint')}</p></div>
        <button type='button' className={styles.artifactToggle} aria-expanded={artifactsOpen} aria-controls='plugin-authoring-artifacts'
          aria-label={t(artifactsOpen ? 'pluginPlatform.authoring.hideArtifacts' : 'pluginPlatform.authoring.showArtifacts')}
          onClick={() => setArtifactsOpen(value => !value)}><Code /><span>{t('pluginPlatform.authoring.artifacts')}</span>
          <Right className={artifactsOpen ? styles.toggleOpen : undefined} /></button>
      </header>
      {!desktop && <Alert type='info' content={t('pluginPlatform.readOnly.body')} />}
      {error && <Alert type='error' content={error} />}
      {!sessionId && preflight && preflight.status !== 'ready' && <Alert type='warning' content={<div>
        <p>{t('pluginPlatform.authoring.moduleNeeded')}</p>
        <Button onClick={() => navigate(`/agent?${selection.kind === 'preset' ? `preset=${encodeURIComponent(selection.presetId)}` : `template=${encodeURIComponent(selection.templateKey)}`}&module=plugin.development`)}>
          {t('pluginPlatform.authoring.configureAgent')}
        </Button>
        <Button onClick={() => setSelection({ ...selection })}>{t('pluginPlatform.authoring.retry')}</Button>
      </div>} />}
      <div className={`${styles.body} ${artifactsOpen ? '' : styles.bodyExpanded}`}>
        <section className={styles.conversation} aria-label={t('pluginPlatform.authoring.conversation')}>
          <div className={styles.conversationHeader}>
            <span className={styles.paneLabel}><MessageOne />{t('pluginPlatform.authoring.conversation')}</span>
            <div className={styles.selectors}>
              {!sessionId ? <GuidAgentSelector presets={presets} officialTemplates={templates} selection={selection}
                isLoading={agents.isLoading} loadError={agents.error} onRetry={agents.refresh} disabled={!desktop || busy}
                onSelectPreset={presetId => setSelection({ kind: 'preset', presetId })}
                onSelectTemplate={templateKey => setSelection({ kind: 'template', templateKey })} />
                : <span className={styles.boundAgent} title={boundAgentLabel}>{boundAgentLabel}</span>}
            </div>
          </div>
          {loading ? <div className={styles.loading}><Spin /></div> : session ? <PreviewProvider persistNamespace={`plugin-authoring:${session.id}`} subscribeGlobalOpen={false}>
            <NomiChat conversation_id={session.id} workspace='' modelSelection={modelSelection} hideSendBox
              currentAgent={session.preset_id ? { presetId: session.preset_id, label: boundAgentLabel } : undefined}
              isProcessing={Boolean(running)} creationEnabled={false} creationTasksEnabled={false} />
          </PreviewProvider> : <div className={styles.empty}>
            <span className={styles.emptyMark}><Magic size={26} /></span>
            <h2>{t('pluginPlatform.authoring.emptyTitle')}</h2><p>{t('pluginPlatform.authoring.emptyBody')}</p>
            {!draftId && !pluginId && <div className={styles.ideas}>{ideas.map(idea => <button key={idea.key} type='button'
              className={styles.idea} disabled={!desktop || busy} onClick={() => {
                setInput(t(`pluginPlatform.authoring.ideas.${idea.key}.prompt`)); inputRef.current?.focus();
              }}>{idea.icon}<span>{t(`pluginPlatform.authoring.ideas.${idea.key}.title`)}</span><Right /></button>)}</div>}
            {draftId && !sessionId && <Button type='primary' loading={busy} disabled={!model || preflight?.status !== 'ready' || !desktop} onClick={() => void openDraft()}>
              {t('pluginPlatform.authoring.openDraft')}
            </Button>}
          </div>}
          <div className={styles.composer}>
            <label className={styles.inputLabel} htmlFor='plugin-requirement'>{t('pluginPlatform.authoring.requirement')}</label>
            <Input.TextArea ref={ref => { inputRef.current = ref?.dom ?? null; }} id='plugin-requirement' value={input} onChange={setInput}
              disabled={!desktop || busy || loading || Boolean(running)} placeholder={t('pluginPlatform.authoring.requirementPlaceholder')}
              autoSize={{ minRows: 2, maxRows: 6 }} onKeyDown={event => {
                if (event.key === 'Enter' && (event.ctrlKey || event.metaKey) && !event.nativeEvent.isComposing) {
                  event.preventDefault(); void send();
                }
              }} />
            <div className={styles.composerTools}>
              <div className={styles.model}><ChatModelSelector providers={modelSelection.providers} currentModel={modelSelection.current_model}
                getAvailableModels={modelSelection.getAvailableModels} onSelectModel={modelSelection.handleSelectModel}
                reasoningEffort={reasoningEffort} reasoningEffortOptions={reasoningOptions}
                reasoningEffortDisabled={!desktop || busy || loading || Boolean(running || paused) || session?.runtime?.state === 'starting'}
                onReasoningEffortChange={reasoningOptions.length ? changeReasoning : undefined}
                disabled={!desktop || busy || loading || Boolean(running || paused)} /></div>
              <div className={styles.actions}>
                {running ? <Button className={styles.sendButton} loading={busy} icon={<PauseOne />} aria-label={t('pluginPlatform.authoring.stop')} title={t('pluginPlatform.authoring.stop')} onClick={() => void stop()}><span className={styles.sendButtonLabel}>{t('pluginPlatform.authoring.stop')}</span></Button>
                  : <Button className={styles.sendButton} type='primary' loading={busy} icon={<Send />}
                    aria-label={t(paused ? 'pluginPlatform.authoring.replyContinue' : 'pluginPlatform.authoring.sendRequirement')}
                    title={t(paused ? 'pluginPlatform.authoring.replyContinue' : 'pluginPlatform.authoring.sendRequirement')}
                    disabled={!desktop || busy || !input.trim() || loading || (!session && (!model || preflight?.status !== 'ready'))} onClick={() => void send()}>
                    <span className={styles.sendButtonLabel}>{t(paused ? 'pluginPlatform.authoring.replyContinue' : 'pluginPlatform.authoring.sendRequirement')}</span>
                  </Button>}
              </div>
            </div>
          </div>
        </section>
        <section id='plugin-authoring-artifacts' className={styles.artifacts} hidden={!artifactsOpen} aria-label={t('pluginPlatform.authoring.artifacts')}>
          <div className={styles.artifactHeader}><span className={styles.paneLabel}><Code />{t('pluginPlatform.authoring.artifacts')}</span></div>
          {session ? <PluginAuthoringArtifacts conversationId={session.id} visible={artifactsOpen} operationDisabled={busy || Boolean(running)} onAuthoringOperation={runAuthoringOperation} /> : artifactEmpty}
        </section>
      </div>
    </div>
  </PluginWorkspace>;
}
