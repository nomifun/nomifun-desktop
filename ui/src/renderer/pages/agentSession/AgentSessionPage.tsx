import React, { Suspense, useEffect, useRef, useState } from 'react';
import { useNavigate, useParams } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import useSWR from 'swr';
import { Alert, Button, Spin } from '@arco-design/web-react';
import { agentPlatform, pluginRuntimes } from '@/common/adapter/ipcBridge';
import { pluginRuntimeProduct } from '@/common/adapter/pluginRuntimeProductBridge';
import type { AgentUiContribution, PluginRuntimeSurfaceLaunchDescriptor } from '@/common/types/pluginRuntimePlatform';
import { agentUiChoiceKey as choiceKey } from '@/common/utils/agentUiChoice';
import PluginRuntimeSurfacePanel from '../plugins/runtime/PluginRuntimeSurfacePanel';
import styles from './AgentSessionPage.module.css';
import { useAgentUiAvailable } from '@/renderer/hooks/agent/useAgentUiAvailable';
import { useAgentUiBindingCache } from '@/renderer/hooks/agent/useAgentUiBindingCache';

const BuiltinAgentSessionPage = React.lazy(() => import('./BuiltinAgentSessionPage'));

async function closeView(surface: PluginRuntimeSurfaceLaunchDescriptor) {
  await pluginRuntimes.closeSurface.invoke({
    plugin_id: surface.plugin_id, surface_session_id: surface.surface_session_id,
    surface_capability: surface.surface_capability,
  });
}

type ViewState = { selection: AgentUiContribution; surface?: PluginRuntimeSurfaceLaunchDescriptor; failed?: boolean };

/** Only presentation is selected here. The backend retains all Session state. */
export function AgentSessionViewHost(props: {
  sessionId: string; children: React.ReactNode; onDraftCreated?: (draftId: string) => void;
}) {
  const available = useAgentUiAvailable();
  return available ? <ExperimentalAgentSessionViewHost {...props} /> : <>{props.children}</>;
}

function ExperimentalAgentSessionViewHost({ sessionId, children, onDraftCreated }: {
  sessionId: string; children: React.ReactNode; onDraftCreated?: (draftId: string) => void;
}) {
  const { t } = useTranslation();
  const updateBindingCache = useAgentUiBindingCache();
  const { data: choices = [], error: catalogError, isLoading: catalogLoading, mutate } = useSWR(
    'agent-catalog/ui/agent-session', () => agentPlatform.agentUiContributions.invoke()
  );
  const { data: preference, error: preferenceError, isValidating: preferenceValidating, mutate: refreshPreference } = useSWR(
    ['agent-session-ui-binding', sessionId], () => agentPlatform.sessions.uiBinding.invoke({ agent_session_id: sessionId }),
    { shouldRetryOnError: false, revalidateOnMount: false }
  );
  const [preferenceRead, setPreferenceRead] = useState(false);
  useEffect(() => {
    // Cached SWR data can render before its deferred mount revalidation starts.
    // Explicitly reread on entry, even on a quick return to the same Session.
    let disposed = false;
    const complete = () => { if (!disposed) setPreferenceRead(true); };
    void refreshPreference().then(complete, complete);
    return () => { disposed = true; };
  }, [refreshPreference]);
  const [choiceResolved, setChoiceResolved] = useState(false);
  const [savingPreference, setSavingPreference] = useState(false);
  const [saveError, setSaveError] = useState(false);
  const preferencePending = useRef(false);
  const [candidate, setCandidate] = useState('');
  const [selected, setSelected] = useState<AgentUiContribution | null>(null);
  const [state, setState] = useState<ViewState | null>(null);
  const [cleanupError, setCleanupError] = useState(false);
  const [revoked, setRevoked] = useState<AgentUiContribution | null>(null);
  const [creatingTemplate, setCreatingTemplate] = useState(false);
  const [templateError, setTemplateError] = useState(false);
  const templatePending = useRef(false);
  const mounted = useRef(false);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const surface = state?.selection === selected ? state?.surface : undefined;
  const failed = state?.selection === selected && state?.failed;
  const available = selected === null || choices.some(choice => choice.capability.id === selected.capability.id &&
    choice.capability.version === selected.capability.version && choice.plugin_id === selected.plugin_id &&
    choice.expected_release_digest === selected.expected_release_digest);
  const remembered = preference?.binding.selection;
  const rememberedAvailable = !remembered || choices.some(choice => choiceKey(choice) === choiceKey(remembered));

  useEffect(() => {
    // Resolve once per route mount. A refresh/late save must not replace a view
    // the user explicitly selected, nor renew a latched in-page revocation.
    if (choiceResolved || !preferenceRead || preferenceValidating || (!preference && !preferenceError) || catalogLoading) return;
    setChoiceResolved(true);
    if (remembered && rememberedAvailable && !preferenceError && !catalogError) {
      const choice = choices.find(item => choiceKey(item) === choiceKey(remembered));
      if (choice) { setSelected({ ...choice }); setCandidate(choiceKey(choice)); }
    }
  }, [choiceResolved, preferenceRead, preference, preferenceError, preferenceValidating, catalogLoading, catalogError, choices, remembered, rememberedAvailable]);

  useEffect(() => {
    if (!selected) return;
    if (!available) { setRevoked(selected); return; }
    if (revoked === selected) return;
    let disposed = false;
    let opened: PluginRuntimeSurfaceLaunchDescriptor | undefined;
    setState(null);
    const release = (view: PluginRuntimeSurfaceLaunchDescriptor) => closeView(view).catch(() => {
      // Also handle a late open after navigation. Do not claim revocation if
      // the close request failed; server-side release checks remain in force.
      setCleanupError(true);
      console.warn('Agent view close failed; server-side revocation is unconfirmed');
    });
    void pluginRuntimes.openSurface.invoke({
      plugin_id: selected.plugin_id,
      agent_session: { agent_session_id: sessionId, expected_release_digest: selected.expected_release_digest, ui_capability: selected.capability },
    }).then(async next => {
      if (disposed) { await release(next); return; }
      opened = next;
      setState({ selection: selected, surface: next });
    }).catch(() => {
      if (!disposed) setState({ selection: selected, failed: true });
    });
    return () => {
      disposed = true;
      if (opened) void release(opened);
    };
  }, [selected, available, sessionId, revoked]);

  const choose = () => {
    if (preferencePending.current) return;
    const choice = choices.find(item => choiceKey(item) === candidate);
    if (choice) { setChoiceResolved(true); setSelected({ ...choice }); }
  };

  const useBuiltin = () => {
    if (preferencePending.current) return;
    setChoiceResolved(true); setSelected(null); setState(null);
  };

  const remember = async (selection: AgentUiContribution | null) => {
    if (!preference || preferencePending.current) return;
    preferencePending.current = true;
    setSavingPreference(true); setSaveError(false);
    try {
      const next = await agentPlatform.putPresetUiBinding.invoke({ preset_id: preference.preset_id, request: {
        expected_binding_version: preference.binding.binding_version, selection,
      } });
      await updateBindingCache(next);
      if (!mounted.current) return;
      await refreshPreference(next, false);
      if (!mounted.current) return;
      setChoiceResolved(true);
      // Remembering an already open page must not discard its local UI state.
      const nextChoice = next.binding.selection;
      setSelected(current => nextChoice && current && current !== revoked && choiceKey(current) === choiceKey(nextChoice)
        ? current : nextChoice ? { ...nextChoice } : null);
      setCandidate(next.binding.selection ? choiceKey(next.binding.selection) : '');
    } catch {
      if (mounted.current) setSaveError(true);
    } finally {
      preferencePending.current = false;
      if (mounted.current) setSavingPreference(false);
    }
  };

  const createTemplate = async () => {
    if (!onDraftCreated || templatePending.current) return;
    templatePending.current = true;
    setCreatingTemplate(true); setTemplateError(false);
    try {
      const draft = await pluginRuntimeProduct.agentSessionTemplate.invoke();
      if (mounted.current) onDraftCreated(draft.id);
    } catch {
      if (mounted.current) setTemplateError(true);
    } finally {
      templatePending.current = false;
      if (mounted.current) setCreatingTemplate(false);
    }
  };

  return <section className={styles.viewHost} aria-label={t('agentSettings.view.title')}>
    <div className={styles.viewControls}>
      <label htmlFor='agent-session-view-choice'>{t('agentSettings.view.title')}</label>
      <select id='agent-session-view-choice' value={candidate} onChange={event => setCandidate(event.target.value)}>
        <option value=''>{t('agentSettings.view.choose')}</option>
        {choices.map(choice => <option key={choiceKey(choice)} value={choiceKey(choice)}>{choice.display_name} — {choice.capability.id}@{choice.capability.version}</option>)}
      </select>
      <Button onClick={choose} disabled={savingPreference || !choices.some(choice => choiceKey(choice) === candidate)}>{t('agentSettings.view.use')}</Button>
      <Button onClick={useBuiltin} disabled={savingPreference || (!selected && choiceResolved)}>{t('agentSettings.view.builtin')}</Button>
      <Button onClick={() => { void mutate(); void refreshPreference(); }} disabled={savingPreference}>{t('agentSettings.actions.retry')}</Button>
      <Button loading={savingPreference} disabled={!preference || savingPreference || !choices.some(choice => choiceKey(choice) === candidate)} onClick={() => {
        const choice = choices.find(item => choiceKey(item) === candidate);
        if (choice) void remember(choice);
      }}>{t('agentSettings.view.remember')}</Button>
      <Button disabled={!remembered || savingPreference} onClick={() => void remember(null)}>{t('agentSettings.view.clearDefault')}</Button>
      {onDraftCreated && <Button loading={creatingTemplate} disabled={creatingTemplate} onClick={() => void createTemplate()}>{t('agentSettings.view.createTemplate')}</Button>}
      <p className={styles.viewHint}>{t('agentSettings.view.consent')}</p>
      {preference && <p className={styles.viewHint}>{t('agentSettings.view.defaultConsent', { name: preference.display_name })}</p>}
      {selected && <span>{t('agentSettings.view.current', { name: selected.display_name })}</span>}
    </div>
    {catalogError && <Alert type='warning' content={t('agentSettings.view.catalogFailed')} />}
    {preferenceError && <Alert type='warning' content={t('agentSettings.view.defaultFailed')} />}
    {saveError && <Alert type='warning' content={t('agentSettings.view.defaultSaveFailed')} />}
    {remembered && !catalogLoading && !rememberedAvailable && <Alert type='warning' content={t('agentSettings.view.defaultUnavailable')} />}
    {templateError && <Alert type='warning' content={t('agentSettings.view.templateFailed')} />}
    {cleanupError && <Alert type='warning' content={t('agentSettings.view.closeFailed')} />}
    {selected ? !available || revoked === selected ? <Alert type='warning' content={t('agentSettings.view.changed')} /> : failed ? (
      <Alert type='error' content={t('agentSettings.view.failed')} />
    ) : surface ? (
      <PluginRuntimeSurfacePanel compact descriptor={surface} displayName={selected.display_name}
        reloading={false} closing={false} onReload={() => setSelected({ ...selected })} onClose={useBuiltin} />
    ) : <div className={styles.loading}><Spin /><span>{t('agentSettings.session.loading')}</span></div> : (
      choiceResolved ? children : <div className={styles.loading}><Spin /><span>{t('agentSettings.session.loading')}</span></div>
    )}
  </section>;
}

export default function AgentSessionPage() {
  const { agentSessionId = '' } = useParams();
  const navigate = useNavigate();
  // Route changes discard UI grants; an old asynchronous response cannot mount
  // in another Session. A remembered exact release is opt-in per Agent; each
  // mount still acquires a fresh, server-validated grant for this Session only.
  return <AgentSessionViewHost key={agentSessionId} sessionId={agentSessionId}
    onDraftCreated={draftId => void navigate(`/plugins/create/${draftId}`)}>
    <Suspense fallback={<Spin />}><BuiltinAgentSessionPage /></Suspense>
  </AgentSessionViewHost>;
}
