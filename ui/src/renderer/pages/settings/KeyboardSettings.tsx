import { Alert, Button, Input, Select, Spin } from '@arco-design/web-react';
import { Write } from '@icon-park/react';
import { useEffect, useId, useMemo, useRef, useState, type CompositionEvent, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { Link } from 'react-router-dom';
import useSWR from 'swr';
import { ipcBridge } from '@/common';
import { keyboard, modelSelectionKey, parseModelSelection, type KeyboardAsrMode, type KeyboardCatalogModel, type KeyboardModel, type KeyboardSelection, type KeyboardSettings as Settings, type KeyboardState, type VoiceKeyboardShortcuts } from '@/common/keyboard';
import { useSystemPermissions } from '@/renderer/hooks/system/useSystemPermissions';
import { deriveKeyboardReadiness } from './keyboardReadiness';
import { useKeyboardSettingsAutosave } from './useKeyboardSettingsAutosave';
import { useKeyboardMicrophoneTest } from './useKeyboardMicrophoneTest';
import { useKeyboard } from '@/renderer/hooks/keyboard/useKeyboard';
import PageHeader from '@/renderer/components/layout/PageHeader';
import { VisualChoice, VisualPanel, VisualRow, VisualStatus, VisualSwitch, VisualTabs } from './components/CodeVisualPrimitives';
import SettingsPageWrapper from './components/SettingsPageWrapper';
import './keyboard-settings.css';

type CloudTestBaseline = { revision: number; text: string; selection: KeyboardSelection | null };
type SettingsTab = 'voice' | 'input' | 'appearance' | 'diagnostics';

function SettingField({ label, hint, children }: { label: string; hint?: ReactNode; children: ReactNode }) {
  return <div className='voice-field'><div className='voice-field__label'>{label}</div>{children}
    {hint && <div className='voice-field__hint'>{hint}</div>}
  </div>;
}
type CloudTestRequest = {
  sequence: number;
  documentId: string | null;
  operationId: string | null;
  active: boolean;
  acknowledged: boolean;
  signature: string;
  baseline: CloudTestBaseline;
  readonly cancelGeneration: number;
};

function ModelSelect({ models, groups, value, label, disabled, onChange }: {
  models: KeyboardCatalogModel[]; value: KeyboardModel | null; label: string; disabled: boolean;
  groups?: Array<{ label: string; models: KeyboardCatalogModel[] }>;
  onChange: (model: KeyboardModel | null) => void;
}) {
  const { t } = useTranslation();
  const selected = modelSelectionKey(value);
  const missing = selected && !models.some((model) => modelSelectionKey(model) === selected);
  return <Select className='keyboard-model-select' aria-label={label} value={selected} disabled={disabled}
    onChange={(key: string) => onChange(parseModelSelection(key))}>
    <Select.Option value=''>{t('settings.keyboard.noModel')}</Select.Option>
    {missing && <Select.Option value={selected} disabled>{t('settings.keyboard.unavailableModel', { model: value?.model })}</Select.Option>}
    {groups ? groups.filter(group => group.models.length).map(group => <Select.OptGroup key={group.label} label={group.label}>
      {group.models.map(model => <Select.Option key={modelSelectionKey(model)} value={modelSelectionKey(model)}>
        {model.label}
      </Select.Option>)}
    </Select.OptGroup>) : models.map((model) => <Select.Option key={modelSelectionKey(model)} value={modelSelectionKey(model)}>
      {model.label}
    </Select.Option>)}
  </Select>;
}

function shortcutKeyLabels(value: string, platform: string | undefined): string[] {
  const raw = value.trim();
  if (!raw) return [];
  const parts = raw.split('+').map(part => part.trim());
  // Display incomplete/unknown input honestly; only the native parser decides
  // whether the unchanged binding is valid when the group is applied.
  if (parts.some(part => !part)) return [raw];
  const mac = platform === 'macos';
  const commandOrControl = mac ? '⌘' : platform === 'windows' || platform === 'linux' ? 'Ctrl' : 'Ctrl/⌘';
  const command = mac ? '⌘' : platform === 'windows' ? 'Win' : 'Super';
  const names: Record<string, string> = {
    commandorcontrol: commandOrControl, cmdorctrl: commandOrControl,
    control: mac ? '⌃' : 'Ctrl', ctrl: mac ? '⌃' : 'Ctrl',
    command, cmd: command, super: command, meta: command,
    shift: mac ? '⇧' : 'Shift', alt: mac ? '⌥' : 'Alt', option: mac ? '⌥' : 'Alt',
    space: 'Space', spacebar: 'Space', return: 'Enter', escape: 'Esc', esc: 'Esc',
    arrowup: '↑', up: '↑', arrowdown: '↓', down: '↓', arrowleft: '←', left: '←', arrowright: '→', right: '→',
    plus: '+', backspace: mac ? '⌫' : 'Backspace', delete: mac ? '⌦' : 'Delete',
  };
  return parts.map(part => names[part.toLowerCase()] ?? (part.length === 1 ? part.toUpperCase() : part));
}

function ShortcutBindings({ shortcuts, platform, disabled, onApply }: {
  shortcuts: VoiceKeyboardShortcuts; platform?: string; disabled: boolean; onApply: (next: VoiceKeyboardShortcuts) => void;
}) {
  const { t } = useTranslation();
  const [edited, setEdited] = useState(shortcuts);
  const [editing, setEditing] = useState<keyof VoiceKeyboardShortcuts | null>(null);
  const previewRefs = useRef<Partial<Record<keyof VoiceKeyboardShortcuts, HTMLButtonElement | null>>>({});
  const returnFocus = useRef<keyof VoiceKeyboardShortcuts | null>(null);
  const bindingId = useId();
  const saved = JSON.stringify(shortcuts);
  useEffect(() => { setEdited(shortcuts); }, [saved]);
  useEffect(() => {
    if (editing === null && returnFocus.current) {
      previewRefs.current[returnFocus.current]?.focus();
      returnFocus.current = null;
    }
  }, [editing]);
  return <>
    <div className='voice-shortcut-grid'>{(['toggle', 'dictate', 'beautify', 'cancel'] as const).map(action => {
      const id = bindingId + '-' + action;
      const keys = shortcutKeyLabels(edited[action], platform);
      const isEditing = editing === action;
      return <div key={action} className='voice-shortcut-field'>
        <label htmlFor={isEditing ? id + '-input' : id}>{t(`settings.keyboard.shortcut_${action}`)}</label>
        <div className='voice-shortcut-control'>
          <button type='button' id={id} className='voice-shortcut-preview' aria-label={t(`settings.keyboard.shortcut_${action}`)}
            aria-describedby={id + '-keys ' + id + '-hint'} aria-hidden={isEditing || undefined} tabIndex={isEditing ? -1 : undefined}
            disabled={disabled || isEditing} title={(keys.length ? keys.join(' + ') : t('settings.keyboard.shortcutNotSet')) + ' · ' + t('settings.keyboard.shortcutEditHint')}
            ref={node => { previewRefs.current[action] = node; }} onClick={() => setEditing(action)}>
            <span className='voice-shortcut-keys' id={id + '-keys'}>
              {keys.length ? keys.map((key, index) => <span className='voice-shortcut-part' key={index}>
                {index > 0 && <span className='voice-shortcut-plus' aria-hidden='true'>+</span>}<kbd>{key}</kbd>
              </span>) : <span className='voice-shortcut-empty'>{t('settings.keyboard.shortcutNotSet')}</span>}
            </span>
            <span className='voice-shortcut-edit-icon' aria-hidden='true'><Write theme='outline' size={13} /></span>
          </button>
          {isEditing && <Input id={id + '-input'} aria-describedby={id + '-hint'} className='keyboard-shortcut-input'
            aria-label={t(`settings.keyboard.shortcut_${action}`)} value={edited[action]} autoFocus
            disabled={disabled} onFocus={event => event.target.select()} onChange={value => setEdited(current => ({ ...current, [action]: value }))}
            onBlur={() => setEditing(null)} onKeyDown={event => {
              if (event.key === 'Enter' && !event.nativeEvent.isComposing) {
                event.preventDefault(); returnFocus.current = action; setEditing(null);
              }
            }} placeholder={t('settings.keyboard.shortcutEmpty')} spellCheck={false} />}
        </div>
        <span id={bindingId + '-' + action + '-hint'}>{t(`settings.keyboard.shortcutHint_${action}`)}</span>
      </div>;
    })}</div>
    <div className='keyboard-form-actions'>
      <Button disabled={disabled || JSON.stringify(edited) === saved} onClick={() => onApply(edited)}>{t('settings.keyboard.applyShortcuts')}</Button>
    </div>
    <details className='voice-disclosure voice-shortcut-guide'><summary>{t('settings.keyboard.shortcutGuide')}</summary>
      <p>{t('settings.keyboard.shortcutFormat')}</p>
    </details>
  </>;
}

function ExternalKeyboardPanel({ state, draft, onChange }: {
  state: KeyboardState; draft: Settings; onChange: (patch: Partial<Settings>) => void;
}) {
  const { t } = useTranslation();
  const status = state.externalKeyboard.state;
  const stopping = status === 'stopping';
  const showDevice = draft.externalKeyboardEnabled || status !== 'disabled';
  const detail = state.externalKeyboard.detail;
  return <VisualPanel title={t('settings.keyboard.externalTitle')} description={t('settings.keyboard.externalHint')}>
    <VisualRow label={t('settings.keyboard.externalEnabled')} description={t('settings.keyboard.externalEnabledHint')}>
      <VisualSwitch label={t('settings.keyboard.externalEnabled')} checked={draft.externalKeyboardEnabled} disabled={stopping}
        onChange={externalKeyboardEnabled => onChange({ externalKeyboardEnabled })} />
    </VisualRow>
    <VisualRow label={t('settings.keyboard.externalStatus')} description={status === 'error' ? undefined : detail}>
      <VisualStatus tone={status === 'error' ? 'danger' : status === 'connected' ? 'success' : stopping || status === 'starting' ? 'info' : 'neutral'}>
        {t(`settings.keyboard.external_${status}`)}
      </VisualStatus>
    </VisualRow>
    {status === 'error' && detail && <Alert type='warning' content={detail} />}
    {showDevice && <>
      {state.device.detail && state.device.detail !== detail && <p className='cv-note'>{state.device.detail}</p>}
      <div className='voice-hardware-keys' role='group' aria-label={t('settings.keyboard.externalKeys')}>
        <VisualRow label={t('settings.keyboard.externalK1')} description={t('settings.keyboard.externalK1Hint')} />
        {(['k2Enabled', 'k3Enabled', 'k4Enabled'] as const).map(key => <VisualRow key={key} label={t(`settings.keyboard.${key}`)}
          description={key === 'k4Enabled' ? t('settings.keyboard.externalK4Hint') : undefined}
          disabled={!draft.externalKeyboardEnabled || stopping}>
          <VisualSwitch label={t(`settings.keyboard.${key}`)} checked={draft[key]} disabled={!draft.externalKeyboardEnabled || stopping}
            onChange={value => onChange({ [key]: value })} />
        </VisualRow>)}
      </div>
      {draft.externalKeyboardEnabled && !draft.k4Enabled && <p className='cv-note'>{t('settings.keyboard.externalDictationDisabled')}</p>}
      {state.device.connected && <details className='voice-disclosure'>
        <summary>{t('settings.keyboard.externalDetails')}</summary>
        <dl className='voice-device-details'>
          <dt>{t('settings.keyboard.externalDevice')}</dt><dd>{[state.device.id, state.device.port].filter(Boolean).join(' · ')}</dd>
          {state.device.firmware && <><dt>{t('settings.keyboard.externalFirmware')}</dt><dd>{state.device.firmware}</dd></>}
        </dl>
      </details>}
    </>}
  </VisualPanel>;
}

export default function KeyboardSettings() {
  const { t } = useTranslation();
  const tabsId = useId();
  const [activeTab, setActiveTab] = useState<SettingsTab>('voice');
  const navigationRef = useRef<HTMLDivElement>(null);
  const selectTab = (tab: SettingsTab) => {
    setActiveTab(tab);
    navigationRef.current?.closest<HTMLElement>('.settings-page-wrapper')?.scrollTo?.({ top: 0 });
  };
  const native = useKeyboard();
  const { data: catalog, error: catalogError, isLoading: catalogLoading, mutate: refreshCatalog } = useSWR(
    native.available ? 'keyboard-model-catalog' : null, keyboard.getCatalog,
    { revalidateOnFocus: false, shouldRetryOnError: false }
  );
  useEffect(() => {
    if (!native.available) return;
    const refresh = () => { void refreshCatalog().catch(() => undefined); };
    const stopChanges = ipcBridge.mode.onProvidersChanged.on(refresh);
    const stopReconnect = ipcBridge.conversation.reconnected.on(refresh);
    return () => { stopChanges(); stopReconnect(); };
  }, [native.available, refreshCatalog]);
  const nativeState = native.state;
  const captureOccupied = !!nativeState && (nativeState.capturePreparing || nativeState.phase !== 'idle'
    || !['stopped', 'error'].includes(nativeState.audioState.state) || !nativeState.microphoneTest.stopped);
  const autosave = useKeyboardSettingsAutosave(nativeState, { captureBusy: captureOccupied });
  const draft = autosave.draft;
  const [dictionaryEditing, setDictionaryEditing] = useState<string | null>(null);
  const state = useMemo(() => nativeState && autosave.saved ? { ...nativeState, settings: autosave.saved } : nativeState, [nativeState, autosave.saved]);
  const permissions = useSystemPermissions(native.available, 'keyboard-settings');
  const permission = permissions.status?.permissions.find(entry => entry.kind === 'microphone');
  const { data: audioDevices, error: audioDevicesError, isValidating: audioDevicesLoading, mutate: refreshAudioDevices } = useSWR(
    native.available ? 'keyboard-audio-devices' : null, keyboard.getAudioDevices,
    { revalidateOnFocus: false, shouldRetryOnError: false }
  );
  const local = useKeyboardMicrophoneTest(nativeState, autosave.saved?.audioDeviceId ?? null);
  const [cloudResult, setCloudResult] = useState<{ state: 'unverified' | 'running' | 'completed' | 'failed' | 'cancelled'; detail?: string }>({ state: 'unverified' });
  const cloudSignature = JSON.stringify([state?.settings.asrMode, state?.settings.asrModel, state?.settings.audioDeviceId,
    draft?.asrMode, draft?.asrModel, draft?.audioDeviceId]);
  const latestCloudSignature = useRef(cloudSignature);
  latestCloudSignature.current = cloudSignature;
  const test = useRef<CloudTestRequest | null>(null);
  useEffect(() => { test.current = null; setCloudResult({ state: 'unverified' }); }, [cloudSignature]);
  const observeTest = (next: KeyboardState) => {
    const request = test.current;
    if (!request || !request.acknowledged || request.signature !== latestCloudSignature.current || next.sequence <= request.sequence) return;
    if (next.cancelGeneration !== request.cancelGeneration) {
      test.current = null;
      setCloudResult(next.error ? { state: 'failed', detail: next.error } : { state: 'cancelled' });
      return;
    }
    if (request.operationId && next.operationId && next.operationId !== request.operationId
      || request.documentId && next.documentId && next.documentId !== request.documentId) {
      test.current = null;
      setCloudResult({ state: 'unverified' });
      return;
    }
    const active = next.capturePreparing || ['recording', 'finalizing'].includes(next.phase)
      || ['starting', 'capturing', 'stopping'].includes(next.audioState.state);
    request.sequence = next.sequence;
    if (active) {
      request.active = true;
      // Preparation can still refer to the old document. Bind only the actual
      // recording operation, whose baseline revision precedes its ASR commit.
      if (!request.operationId && next.operationId && next.documentId) {
        request.documentId = next.documentId;
        request.operationId = next.operationId;
        request.baseline = { revision: next.revision, text: next.text,
          selection: next.dictationSelection };
      }
      setCloudResult({ state: 'running' });
    } else if (request.active || next.error) {
      const baseline = request.baseline;
      const start = baseline.selection ? Math.min(baseline.selection.anchor, baseline.selection.focus) : baseline.text.length;
      const end = baseline.selection ? Math.max(baseline.selection.anchor, baseline.selection.focus) : baseline.text.length;
      const prefix = baseline.text.slice(0, start);
      const suffix = baseline.text.slice(end);
      const submitted = next.text.startsWith(prefix) && next.text.endsWith(suffix) && next.text.length >= prefix.length + suffix.length
        ? next.text.slice(prefix.length, next.text.length - suffix.length) : '';
      const committed = request.operationId && next.documentId === request.documentId && next.revision > baseline.revision && submitted.trim();
      setCloudResult(next.error ? { state: 'failed', detail: next.error }
        : committed ? { state: 'completed', detail: submitted }
        : { state: 'failed', detail: t('settings.keyboard.cloudEmpty') });
      test.current = null;
    }
  };
  useEffect(() => { if (state) observeTest(state); }, [state]);
  const startTest = async () => {
    const baseline = native.getCurrentState();
    if (!baseline) return;
    setCloudResult({ state: 'running' });
    const request: CloudTestRequest = { sequence: baseline.sequence, documentId: null, operationId: null, active: false,
      acknowledged: false, signature: cloudSignature, cancelGeneration: baseline.cancelGeneration,
      baseline: { revision: baseline.revision, text: baseline.text, selection: baseline.dictationSelection } };
    test.current = request;
    const next = await native.action({ action: 'test_start' });
    if (test.current !== request || request.signature !== latestCloudSignature.current) return;
    request.acknowledged = true;
    if (next) { observeTest(next); const latest = native.getCurrentState(); if (latest && latest.sequence > next.sequence) observeTest(latest); }
    else { test.current = null; setCloudResult({ state: 'failed', detail: t('settings.keyboard.cloudStartFailed') }); }
  };
  const asrMode = draft?.asrMode ?? 'streaming';
  const asrModelGroups = [
    { label: t('settings.keyboard.asrModeStreaming'), models: catalog?.asr ?? [] },
    { label: t('settings.keyboard.asrModePlatform'), models: catalog?.platformAsr ?? [] },
  ];
  const asrModels = asrModelGroups.flatMap(group => group.models);
  const change = <K extends keyof Settings>(key: K, value: Settings[K], mode: 'immediate' | 'debounced' = 'immediate') => {
    autosave.change({ [key]: value } as Partial<Settings>, mode);
  };
  const changeAsrMode = (mode: KeyboardAsrMode) => {
    if (!draft) return;
    const models = mode === 'platform' ? catalog?.platformAsr ?? [] : catalog?.asr ?? [];
    autosave.change({ asrMode: mode,
      asrModel: models.some(model => modelSelectionKey(model) === modelSelectionKey(draft.asrModel)) ? draft.asrModel : null });
  };
  const changeAsrModel = (model: KeyboardModel | null) => {
    if (!draft) return;
    if (!model) { change('asrModel', null); return; }
    const key = modelSelectionKey(model);
    const mode = catalog?.asr.some(candidate => modelSelectionKey(candidate) === key) ? 'streaming'
      : catalog?.platformAsr.some(candidate => modelSelectionKey(candidate) === key) ? 'platform' : null;
    // Both mode and exact reference belong to one user choice and one native save.
    if (mode) autosave.change({ asrMode: mode, asrModel: model });
  };
  const textEvents = (field: 'language' | 'dictionary' | 'beautifyInstruction') => ({
    onBlur: () => { if (field === 'dictionary') setDictionaryEditing(null); void autosave.flush(); },
    onCompositionStart: () => autosave.compositionStart(field),
    onCompositionEnd: (event: CompositionEvent<HTMLInputElement | HTMLTextAreaElement>) => {
      const value = event.currentTarget.value;
      if (field === 'dictionary') { setDictionaryEditing(value); autosave.compositionEnd(field, value.split('\n')); }
      else autosave.compositionEnd(field, value);
    },
  });
  const recognitionBusy = captureOccupied || local.active;

  const readiness = state && draft ? deriveKeyboardReadiness({ state, draft, catalog, audioDevices, permission: permission?.state, saving: autosave.pending, saveFailed: autosave.status === 'failed' }) : null;
  const localBlocked = [
    ...(readiness?.permissionBlocked ? ['permission'] : []),
    ...(!readiness?.deviceAvailable ? ['device'] : []),
    ...(native.busy || captureOccupied || local.active ? ['busy'] : []),
    ...(autosave.status === 'failed' ? ['saveFailed'] : autosave.pending ? ['saving'] : []),
  ];
  const cloudBlocked = [...(readiness?.cloudBlockers ?? []), ...(native.busy || captureOccupied || local.active ? ['busy'] : [])];
  const localVerified = local.result.state === 'completed' && local.result.stopped && local.result.callbackObserved && local.result.samples > 0 && local.result.nonzero > 0;
  const localMessage = local.result.state === 'completed' ? !local.result.stopped ? 'localStopping'
    : !local.result.callbackObserved || !local.result.samples ? 'localNoSamples' : !local.result.nonzero ? 'localSilent' : 'localPassed'
    : local.result.state === 'failed' ? 'localFailed' : local.result.state === 'cancelled' ? 'localCancelled'
    : local.result.state === 'starting' ? 'localStarting' : local.result.state === 'listening' ? 'localListening'
    : local.result.state === 'stopping' ? 'localStopping' : 'localUnverified';
  const blockerText = (reason: string) => t(`settings.keyboard.block_${reason}` as 'settings.keyboard.block_busy');

  const saveMessage = t(autosave.status === 'failed' ? 'settings.keyboard.autosaveFailed' : autosave.deferred ? 'settings.keyboard.autosaveDeferred'
    : autosave.status === 'saving' ? 'settings.keyboard.autosaveSaving' : autosave.status === 'pending' ? 'settings.keyboard.autosavePending'
    : autosave.status === 'saved' ? 'settings.keyboard.autosaveSaved' : 'settings.keyboard.saveLabel');
  const localStatusText = t(('settings.keyboard.' + localMessage) as 'settings.keyboard.localUnverified');
  const permissionDescription = t(('settings.keyboard.permission_' + (permission?.state ?? 'unknown')) as 'settings.keyboard.permission_unknown');
  const audioStatusText = t(('settings.keyboard.audio_' + (state?.audioState.state ?? 'stopped')) as 'settings.keyboard.audio_stopped');
  const cloudActive = cloudResult.state === 'running' && !!test.current;
  const cloudStoppable = !!state && (state.capturePreparing || state.phase === 'recording' || ['starting', 'capturing'].includes(state.audioState.state));
  const stopCloudTest = () => { if (state) void native.action({ action: 'test_stop', documentId: state.documentId, revision: state.revision }); };
  const recentError = state?.error ?? '';
  const detailIndex = recentError.indexOf('详情：');
  const recentSummary = detailIndex < 0 ? recentError : recentError.slice(0, detailIndex).trimEnd();
  const recentDetail = detailIndex < 0 ? '' : recentError.slice(detailIndex + '详情：'.length).trim();
  const showDiagnostics = () => selectTab('diagnostics');

  return <SettingsPageWrapper className='voice-settings-page'>
    <PageHeader title={t('settings.keyboard.title')} description={t('settings.keyboard.pageHint')}
      actions={<>
        {state && <section role='status' aria-live='polite' aria-label={t('settings.keyboard.saveStatus')}
          className='voice-save-status' data-state={autosave.status}>
          <span className='voice-save-status__dot' aria-hidden='true' /><span>{saveMessage}</span>
          {autosave.status === 'failed' && <Button size='mini' onClick={() => { void autosave.retry(); }}>{t('settings.keyboard.retrySave')}</Button>}
        </section>}
        <Button size='small' loading={native.busy}
          onClick={() => { void native.refresh(); void refreshCatalog(); void refreshAudioDevices(); void permissions.refresh(); }}
          disabled={!native.available}>{t('settings.keyboard.refresh')}</Button>
        <Button size='small' type='primary' disabled={!state || native.busy}
          onClick={() => void native.action({ action: 'open' })}>{t('settings.keyboard.open')}</Button>
      </>} />
    {!native.available && <Alert type='info' content={t('settings.keyboard.desktopOnly')} />}
    {native.error && <Alert type='error' content={native.error} />}
    {native.available && !state && !native.error && <div className='keyboard-loading' role='status'><Spin />{t('settings.keyboard.loading')}</div>}
    {state && draft && readiness && <>
      {autosave.error && <Alert type='error' content={autosave.error} />}
      {native.busy && <p className='voice-context-note' role='status'>{t('settings.keyboard.block_busy')}</p>}
      {recentError && activeTab !== 'diagnostics' && <div className='voice-notice' role='alert'>
        <div><span className='voice-eyebrow'>{t('settings.keyboard.hardwareErrorTitle')}</span><p>{recentSummary}</p></div>
        <Button size='small' onClick={showDiagnostics}>{t('settings.keyboard.recentErrorAction')}</Button>
      </div>}
      <div className='voice-settings-navigation' ref={navigationRef}>
        <VisualTabs id={tabsId} label={t('settings.keyboard.tabsLabel')} activeKey={activeTab}
          onChange={key => selectTab(key as SettingsTab)} items={[
            { key: 'voice', label: t('settings.keyboard.tabVoice') },
            { key: 'input', label: t('settings.keyboard.tabInput') },
            { key: 'appearance', label: t('settings.keyboard.tabAppearance') },
            { key: 'diagnostics', label: t('settings.keyboard.tabDiagnostics'), dot: local.active || cloudActive },
          ]} />
        {(local.active || cloudActive) && <div className='voice-test-activity' role='status'>
          <span>{local.active ? localStatusText : t('settings.keyboard.cloudRunning')}</span>
          {activeTab !== 'diagnostics' && <>
            <Button size='mini' onClick={showDiagnostics}>{t('settings.keyboard.diagnosticsAction')}</Button>
            <Button size='mini' disabled={local.active ? local.result.state === 'stopping' : !cloudStoppable}
              onClick={local.active ? () => void local.stop() : stopCloudTest}>
              {t(local.active ? 'settings.keyboard.localStop' : 'settings.keyboard.testStop')}
            </Button>
          </>}
        </div>}
      </div>
      <div role='tabpanel' id={tabsId + '-panel'} aria-labelledby={tabsId + '-tab-' + activeTab} className='voice-settings-content'>
        <div hidden={activeTab !== 'voice'} className='voice-pane'>
          <div className='voice-recognition-grid'>
            <VisualPanel title={t('settings.keyboard.recognitionTitle')} description={t('settings.keyboard.recognitionHint')}
              className='voice-recognition-card' action={<Link to='/models'>{t('settings.keyboard.manageModels')}</Link>}>
              {catalogError && <Alert type='error' content={String(catalogError)} />}
              {recognitionBusy && <p className='voice-context-note'>{t('settings.keyboard.recognitionEditingBusy')}</p>}
              {(catalogLoading || !catalog) && <p className='voice-context-note'>{t('settings.keyboard.catalogLoading')}</p>}
              {readiness.modelCatalogueEmpty && <Alert type='info' content={t('settings.keyboard.emptyAsr')} />}
              <SettingField label={t('settings.keyboard.asrMode')} hint={t('settings.keyboard.asrModeHint')}>
                <VisualChoice label={t('settings.keyboard.asrMode')} value={asrMode} disabled={recognitionBusy || catalogLoading || !catalog}
                  options={[
                    { value: 'streaming', label: t('settings.keyboard.asrModeStreaming') },
                    { value: 'platform', label: t('settings.keyboard.asrModePlatform') },
                  ]} onChange={changeAsrMode} />
              </SettingField>
              <SettingField label={t('settings.keyboard.asrModel')} hint={t('settings.keyboard.asrModelHint')}>
                <ModelSelect models={asrModels} groups={asrModelGroups} value={draft.asrModel} label={t('settings.keyboard.asrModel')}
                  disabled={recognitionBusy || catalogLoading || !catalog} onChange={changeAsrModel} />
              </SettingField>
              <details className='voice-disclosure'>
                <summary><span>{t('settings.keyboard.preferencesTitle')}</span><span className='voice-disclosure__meta'>{t('settings.keyboard.preferencesHint')}</span></summary>
                <SettingField label={t('settings.keyboard.language')} hint={t('settings.keyboard.languageHint')}>
                  <Input aria-label={t('settings.keyboard.language')} value={draft.language} disabled={recognitionBusy}
                    {...textEvents('language')} onChange={value => change('language', value, 'debounced')} className='keyboard-short-input' />
                </SettingField>
                <SettingField label={t('settings.keyboard.dictionary')}>
                  <Input.TextArea value={dictionaryEditing ?? draft.dictionary.join('\n')} disabled={recognitionBusy}
                    {...textEvents('dictionary')} onFocus={() => setDictionaryEditing(draft.dictionary.join('\n'))}
                    onChange={value => { setDictionaryEditing(value); change('dictionary', value.split('\n'), 'debounced'); }}
                    autoSize={{ minRows: 3, maxRows: 7 }} aria-label={t('settings.keyboard.dictionary')}
                    placeholder={t('settings.keyboard.dictionaryHint')} />
                </SettingField>
              </details>
            </VisualPanel>
            <VisualPanel title={t('settings.keyboard.microphoneTitle')} description={t('settings.keyboard.inputDeviceHint')} className='voice-microphone-card'
              action={<Button size='mini' loading={audioDevicesLoading} onClick={() => void refreshAudioDevices()}>{t('settings.keyboard.refreshDevices')}</Button>}>
              {audioDevicesError && <Alert type='error' content={String(audioDevicesError)} />}
              {audioDevicesLoading && <p className='voice-context-note'>{t('settings.keyboard.inputsLoading')}</p>}
              {!audioDevicesLoading && !readiness.deviceAvailable && <Alert type='warning' content={t('settings.keyboard.block_device')} />}
              <SettingField label={t('settings.keyboard.audioDevice')}>
                <Select className='keyboard-model-select' aria-label={t('settings.keyboard.audioDevice')} value={draft.audioDeviceId ?? ''}
                  disabled={recognitionBusy || audioDevicesLoading} onChange={(value: string) => change('audioDeviceId', value || null)}>
                  <Select.Option value=''>{t('settings.keyboard.systemDefault')}</Select.Option>
                  {draft.audioDeviceId && !audioDevices?.some(device => device.id === draft.audioDeviceId) &&
                    <Select.Option value={draft.audioDeviceId} disabled>{t('settings.keyboard.unavailableAudioDevice')}</Select.Option>}
                  {(audioDevices ?? []).map(device => <Select.Option key={device.id} value={device.id}>
                    {device.label}{device.isDefault ? ' · ' + t('settings.keyboard.defaultDevice') : ''}
                  </Select.Option>)}
                </Select>
              </SettingField>
              <VisualRow label={t('settings.keyboard.microphoneState')} description={state.audioState.detail}>
                <VisualStatus tone={state.audioState.state === 'error' ? 'danger' : state.audioState.state === 'stopped' ? 'neutral' : 'info'}>
                  {audioStatusText}
                </VisualStatus>
              </VisualRow>
              {readiness.permissionBlocked && <Alert type='warning' content={permissionDescription} />}
              <div className='voice-panel-footer'><span>{t(localVerified && readiness.deviceAvailable ? 'settings.keyboard.localPassed' : 'settings.keyboard.localUnverified')}</span>
                <Button size='small' onClick={showDiagnostics}>{t('settings.keyboard.checkMicrophone')}</Button>
              </div>
            </VisualPanel>
          </div>
        </div>

        <div hidden={activeTab !== 'input'} className='voice-pane'>
          <p className='voice-pane-intro'>{t('settings.keyboard.inputMethodsHint')}</p>
          <div className='voice-input-grid'>
          <VisualPanel title={t('settings.keyboard.keysTitle')} description={t('settings.keyboard.keysHint')} className='voice-shortcuts-card'
            action={<VisualStatus tone={state.shortcutState?.registered ? 'success' : 'neutral'}>
              {t(state.shortcutState?.registered ? 'settings.keyboard.ready' : 'settings.keyboard.stopped')}
            </VisualStatus>}>
            <VisualRow label={t('settings.keyboard.enabled')} description={t('settings.keyboard.enabledHint')}>
              <VisualSwitch label={t('settings.keyboard.enabled')} checked={draft.enabled} onChange={value => change('enabled', value)} />
            </VisualRow>
            <ShortcutBindings shortcuts={draft.shortcuts}
              platform={permissions.status?.platform ?? (typeof window === 'undefined' ? undefined : (window as Window & { __os?: string }).__os)}
              disabled={native.busy || recognitionBusy}
              onApply={shortcuts => change('shortcuts', shortcuts)} />
            {state.shortcutState?.detail && <p className='voice-context-note'>{state.shortcutState.detail}</p>}
            <p className='voice-context-note'>{t('settings.keyboard.disableHint')}</p>
          </VisualPanel>
          <ExternalKeyboardPanel state={state} draft={draft} onChange={patch => autosave.change(patch)} />
          </div>
        </div>

        <div hidden={activeTab !== 'appearance'} className='voice-pane'>
          <div className='voice-appearance-grid'>
            <VisualPanel title={t('settings.keyboard.windowTitle')}>
              <div className='voice-subtitle-preview' aria-label={t('settings.keyboard.subtitlePreview')}>
                <span className='voice-eyebrow'>{t('settings.keyboard.subtitlePreview')}</span>
                <p style={{ fontSize: draft.subtitleFontSize + 'px' }}>{t('settings.keyboard.subtitlePreviewText')}</p>
                <small>{t('settings.keyboard.subtitlePreviewHint')}</small>
              </div>
              <VisualRow label={t('settings.keyboard.subtitleFontSize')} description={t('settings.keyboard.subtitleFontSizeHint')}>
                <Select className='keyboard-font-select' aria-label={t('settings.keyboard.subtitleFontSize')} value={draft.subtitleFontSize}
                  onChange={(value: number) => change('subtitleFontSize', value)}>
                  {Array.from({ length: 21 }, (_, index) => index + 12).map(size => <Select.Option key={size} value={size}>{size}px</Select.Option>)}
                </Select>
              </VisualRow>
              <VisualRow label={t('settings.keyboard.alwaysOnTop')}>
                <VisualSwitch label={t('settings.keyboard.alwaysOnTop')} checked={draft.alwaysOnTop} onChange={value => change('alwaysOnTop', value)} />
              </VisualRow>
              <div className='keyboard-form-actions' role='group' aria-label={t('settings.keyboard.subtitleActions')}>
                <Button disabled={native.busy} onClick={() => void native.action({ action: 'reset_position' })}>{t('settings.keyboard.resetPosition')}</Button>
              </div>
            </VisualPanel>
            <VisualPanel title={t('settings.keyboard.beautifyTitle')} description={t('settings.keyboard.beautifyHint')}>
              {catalogError && <Alert type='error' content={String(catalogError)} />}
              {(catalogLoading || !catalog) && <p className='voice-context-note'>{t('settings.keyboard.catalogLoading')}</p>}
              <SettingField label={t('settings.keyboard.textModel')} hint={t('settings.keyboard.textModelOptional')}>
                <ModelSelect models={catalog?.text ?? []} value={draft.textModel} label={t('settings.keyboard.textModel')}
                  disabled={catalogLoading || !catalog} onChange={value => change('textModel', value)} />
              </SettingField>
              <details className='voice-disclosure'>
                <summary>{t('settings.keyboard.beautifyAdvanced')}</summary>
                <SettingField label={t('settings.keyboard.instruction')} hint={t('settings.keyboard.instructionHint')}>
                  <Input.TextArea value={draft.beautifyInstruction} {...textEvents('beautifyInstruction')}
                    onChange={value => change('beautifyInstruction', value, 'debounced')} autoSize={{ minRows: 4, maxRows: 8 }}
                    aria-label={t('settings.keyboard.instruction')} />
                </SettingField>
              </details>
            </VisualPanel>
          </div>
        </div>

        <div hidden={activeTab !== 'diagnostics'} className='voice-pane'>
          <p className='voice-pane-intro'>{t('settings.keyboard.diagnosticsHint')}</p>
          {recentError && <VisualPanel title={t('settings.keyboard.hardwareErrorTitle')}>
            <div className='voice-error-content'><p>{recentSummary}</p>
              {recentDetail && <details className='voice-disclosure'><summary>{t('settings.keyboard.errorDetails')}</summary><p>{recentDetail}</p></details>}
            </div>
          </VisualPanel>}
          <div className='voice-diagnostics-grid'>
            <VisualPanel title={t('settings.keyboard.localTitle')} description={t('settings.keyboard.testPrivacyLocal')} className='voice-test-card'>
              <span className='voice-eyebrow voice-test-step'>{t('settings.keyboard.localStep')}</span>
              <VisualRow label={t('settings.keyboard.permissionTitle')} description={permissionDescription}>
                <div className='keyboard-inline-actions'>
                  {permission?.can_request && <Button size='small' disabled={permissions.loading} onClick={() => void permissions.request('microphone')}>{t('settings.keyboard.requestMicrophone')}</Button>}
                  {permission?.can_open_settings && <Button size='small' disabled={permissions.loading} onClick={() => void permissions.openSettings('microphone')}>{t('settings.keyboard.openMicrophoneSettings')}</Button>}
                </div>
              </VisualRow>
              {permissions.loading && <p className='voice-context-note' role='status'>{t('settings.keyboard.permissionLoading')}</p>}
              {permissions.error && <Alert type='warning' content={permissions.error} />}
              <div className='keyboard-form-actions'>
                <Button type='primary' disabled={localBlocked.length > 0} onClick={() => void local.start()}>{t('settings.keyboard.localStart')}</Button>
                <Button disabled={!local.active || local.result.state === 'stopping'} onClick={() => void local.stop()}>{t('settings.keyboard.localStop')}</Button>
              </div>
              {localBlocked.length > 0 && <ul className='keyboard-blockers'>{localBlocked.map(reason => <li key={reason}>{blockerText(reason)}</li>)}</ul>}
              <div className='voice-test-result' role='status' aria-live='polite'>
                <VisualStatus tone={localVerified ? 'success' : local.result.state === 'failed' ? 'danger' : 'neutral'}>{localStatusText}</VisualStatus>
                {local.result.detail && <p>{local.result.detail}</p>}
                {local.result.callbackObserved && <p>{t('settings.keyboard.localCallback')}</p>}
              </div>
              {local.result.id && <div className='keyboard-meter'>
                <meter aria-label={t('settings.keyboard.localLevel')} min={0} max={1} value={Math.min(1, Math.max(0, local.result.peak))} />
                <span>{t('settings.keyboard.localStats', { seconds: (local.result.elapsedMs / 1000).toFixed(1), samples: local.result.samples })}</span>
              </div>}
            </VisualPanel>
            <VisualPanel title={t('settings.keyboard.cloudTitle')} description={t('settings.keyboard.testPrivacyCloud')} className='voice-test-card'>
              <span className='voice-eyebrow voice-test-step'>{t('settings.keyboard.cloudStep')}</span>
              <p className='voice-context-note'>{t(asrMode === 'platform' ? 'settings.keyboard.testHintPlatform' : 'settings.keyboard.testHint')}</p>
              <div className='keyboard-form-actions'>
                <Button type='primary' disabled={cloudBlocked.length > 0} onClick={() => void startTest()}>{t('settings.keyboard.testStart')}</Button>
                <Button disabled={!cloudStoppable} onClick={stopCloudTest}>{t('settings.keyboard.testStop')}</Button>
              </div>
              {cloudBlocked.length > 0 && <ul className='keyboard-blockers'>{cloudBlocked.map(reason => <li key={reason}>{blockerText(reason)}</li>)}</ul>}
              <div className='voice-test-result' role='status' aria-live='polite'>
                <VisualStatus tone={cloudResult.state === 'completed' ? 'success' : cloudResult.state === 'failed' ? 'danger' : 'neutral'}>
                  {t(cloudResult.state === 'completed' ? 'settings.keyboard.cloudPassed' : cloudResult.state === 'running' ? 'settings.keyboard.cloudRunning'
                    : cloudResult.state === 'failed' ? 'settings.keyboard.cloudFailed' : cloudResult.state === 'cancelled' ? 'settings.keyboard.cloudCancelled' : 'settings.keyboard.cloudUnverified')}
                </VisualStatus>
                {cloudResult.detail && <p>{cloudResult.detail}</p>}
              </div>
            </VisualPanel>
          </div>
        </div>
      </div>
    </>}
  </SettingsPageWrapper>;
}
