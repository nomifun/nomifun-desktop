import { useEffect, useId, useRef, useState } from 'react';
import { Alert, Button, Spin } from '@arco-design/web-react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import useSWR from 'swr';
import { agentPlatform } from '@/common/adapter/ipcBridge';
import type { AgentPresetSummary } from '@/common/types/agentPlatform';
import { agentUiChoiceKey } from '@/common/utils/agentUiChoice';
import { emitter } from '@/renderer/utils/emitter';
import styles from './AgentPageSettings.module.css';
import { useAgentUiAvailable } from '@/renderer/hooks/agent/useAgentUiAvailable';
import { useAgentUiBindingCache } from '@/renderer/hooks/agent/useAgentUiBindingCache';

/** Configure presentation before the first Session, through the existing APIs. */
export default function AgentPageSettings({ preset, busy, dirty }: {
  preset: AgentPresetSummary; busy: boolean; dirty: boolean;
}) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const agentUiAvailable = useAgentUiAvailable();
  const updateBindingCache = useAgentUiBindingCache();
  const selectId = useId();
  const { data, error, isValidating, mutate } = useSWR(
    ['agent-preset-ui-binding', preset.preset_id],
    () => agentPlatform.presetUiBinding.invoke({ preset_id: preset.preset_id }),
    { shouldRetryOnError: false }
  );
  const { data: choices = [], error: catalogError, isLoading: catalogLoading, mutate: refreshCatalog } = useSWR(
    agentUiAvailable ? 'agent-catalog/ui/agent-session' : null, () => agentPlatform.agentUiContributions.invoke()
  );
  const [candidate, setCandidate] = useState<string>();
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState(false);
  const [opening, setOpening] = useState(false);
  const [openError, setOpenError] = useState(false);
  const [createdSessionId, setCreatedSessionId] = useState<string>();
  const pending = useRef(false);
  const mounted = useRef(false);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const saved = data?.binding.selection;
  const savedKey = saved ? agentUiChoiceKey(saved) : '';
  const selectedKey = candidate ?? savedKey;
  const selected = choices.find(choice => agentUiChoiceKey(choice) === selectedKey);
  const changed = selectedKey !== savedKey;
  const unavailable = !!saved && !catalogLoading && !choices.some(choice => agentUiChoiceKey(choice) === savedKey);
  const blocked = busy || saving || opening;

  const save = async () => {
    if (!data || blocked || pending.current || saveError || !changed || (selectedKey && (!selected || catalogError))) return;
    pending.current = true; setSaving(true);
    try {
      const next = await agentPlatform.putPresetUiBinding.invoke({ preset_id: preset.preset_id, request: {
        expected_binding_version: data.binding.binding_version, selection: selected ?? null,
      } });
      await updateBindingCache(next);
      if (!mounted.current) return;
      await mutate(next, false);
      if (mounted.current) setCandidate(undefined);
    } catch {
      if (mounted.current) setSaveError(true);
    } finally {
      pending.current = false;
      if (mounted.current) setSaving(false);
    }
  };

  const refresh = async () => {
    if (blocked || pending.current) return;
    try {
      await Promise.all([mutate(), refreshCatalog()]);
      if (mounted.current) setSaveError(false);
    } catch { /* SWR exposes the failed read; never retry a mutation here. */ }
  };

  const openPage = async () => {
    if (blocked || pending.current || dirty || !preset.current_stable_revision || !data || changed || saveError) return;
    pending.current = true; setOpening(true); setOpenError(false);
    try {
      // No initial message, model override, new execution owner or Surface grant.
      const sessionId = createdSessionId ?? (await agentPlatform.sessions.create.invoke({
        preset_id: preset.preset_id, title: preset.display_name,
      })).agent_session_id;
      emitter.emit('chat.history.refresh');
      if (!mounted.current) return;
      setCreatedSessionId(sessionId);
      await navigate(`/agent-sessions/${encodeURIComponent(sessionId)}`);
    } catch {
      if (mounted.current) setOpenError(true);
    } finally {
      pending.current = false;
      if (mounted.current) setOpening(false);
    }
  };

  return <section className={styles.pageSettings} aria-label={t('agentSettings.page.title')}>
    <h3>{t('agentSettings.page.title')}</h3>
    <p>{t('agentSettings.page.hint')}</p>
    {error && <Alert type='warning' content={t('agentSettings.view.defaultFailed')} />}
    {catalogError && <Alert type='warning' content={t('agentSettings.view.catalogFailed')} />}
    {saveError && <Alert type='warning' content={t('agentSettings.view.defaultSaveFailed')} />}
    {unavailable && <Alert type='warning' content={t('agentSettings.view.defaultUnavailable')} />}
    {!data && !error && <Spin />}
    {data && <>
      <label htmlFor={selectId}>{t('agentSettings.page.default')}</label>
      <select id={selectId} value={selectedKey} disabled={blocked} onChange={event => setCandidate(event.target.value)}>
        <option value=''>{t('agentSettings.page.builtin')}</option>
        {selectedKey && !selected && <option value={selectedKey} disabled>{t('agentSettings.page.missing')}</option>}
        {choices.map(choice => <option key={agentUiChoiceKey(choice)} value={agentUiChoiceKey(choice)}>
          {choice.display_name} — {choice.capability.id}@{choice.capability.version}
        </option>)}
      </select>
      {selected && <code>{selected.expected_release_digest}</code>}
      <p>{t('agentSettings.view.defaultConsent', { name: data.display_name })}</p>
    </>}
    <div className={styles.actions}>
      <Button onClick={() => void refresh()} disabled={blocked || isValidating}>{t('agentSettings.actions.retry')}</Button>
      <Button onClick={() => void save()} loading={saving}
        disabled={blocked || !data || !changed || saveError || (!!selectedKey && (!selected || !!catalogError))}>
        {t('agentSettings.page.save')}
      </Button>
    </div>
    <div className={styles.launch}>
      <p>{t('agentSettings.page.openHint')}</p>
      {(dirty || !preset.current_stable_revision) && <p role='status'>{t('agentSettings.page.saveAgentFirst')}</p>}
      {changed && <p role='status'>{t('agentSettings.page.savePageFirst')}</p>}
      {openError && <Alert type='warning' content={t('agentSettings.page.openFailed')} />}
      <Button type='primary' onClick={() => void openPage()} loading={opening}
        disabled={blocked || dirty || !preset.current_stable_revision || !data || changed || saveError}>
        {t(createdSessionId ? 'agentSettings.page.continue' : 'agentSettings.page.open')}
      </Button>
    </div>
  </section>;
}
