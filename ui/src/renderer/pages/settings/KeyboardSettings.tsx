import { Alert, Button, Input, Select, Spin } from '@arco-design/web-react';
import { useEffect, useMemo, useRef, useState, type CompositionEvent } from 'react';
import { useTranslation } from 'react-i18next';
import { Link } from 'react-router-dom';
import useSWR from 'swr';
import { keyboard, modelSelectionKey, parseModelSelection, type KeyboardAsrMode, type KeyboardCatalogModel, type KeyboardModel, type KeyboardSelection, type KeyboardSettings as Settings, type KeyboardState } from '@/common/keyboard';
import { useSystemPermissions } from '@/renderer/hooks/system/useSystemPermissions';
import { deriveKeyboardReadiness } from './keyboardReadiness';
import { useKeyboardSettingsAutosave } from './useKeyboardSettingsAutosave';
import { useKeyboardMicrophoneTest } from './useKeyboardMicrophoneTest';
import { useKeyboard } from '@/renderer/hooks/keyboard/useKeyboard';
import PageHeader from '@/renderer/components/layout/PageHeader';
import { VisualPanel, VisualRow, VisualStatus, VisualSwitch } from './components/CodeVisualPrimitives';
import SettingsPageWrapper from './components/SettingsPageWrapper';
import './keyboard-settings.css';

type CloudTestBaseline = { revision: number; text: string; selection: KeyboardSelection | null };
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

function ModelSelect({ models, value, label, disabled, onChange }: {
  models: KeyboardCatalogModel[]; value: KeyboardModel | null; label: string; disabled: boolean;
  onChange: (model: KeyboardModel | null) => void;
}) {
  const { t } = useTranslation();
  const selected = modelSelectionKey(value);
  const missing = selected && !models.some((model) => modelSelectionKey(model) === selected);
  return <Select className='keyboard-model-select' aria-label={label} value={selected} disabled={disabled}
    onChange={(key: string) => onChange(parseModelSelection(key))}>
    <Select.Option value=''>{t('settings.keyboard.noModel')}</Select.Option>
    {missing && <Select.Option value={selected} disabled>{t('settings.keyboard.unavailableModel', { model: value?.model })}</Select.Option>}
    {models.map((model) => <Select.Option key={modelSelectionKey(model)} value={modelSelectionKey(model)}>
      {model.label}
    </Select.Option>)}
  </Select>;
}

export default function KeyboardSettings() {
  const { t } = useTranslation();
  const native = useKeyboard();
  const { data: catalog, error: catalogError, isLoading: catalogLoading, mutate: refreshCatalog } = useSWR(
    native.available ? 'keyboard-model-catalog' : null, keyboard.getCatalog,
    { revalidateOnFocus: false, shouldRetryOnError: false }
  );
  const nativeState = native.state;
  const captureOccupied = !!nativeState && (nativeState.capturePreparing || nativeState.phase !== 'idle'
    || !['stopped', 'error'].includes(nativeState.audioState.state) || !nativeState.microphoneTest.stopped);
  const autosave = useKeyboardSettingsAutosave(nativeState, { captureBusy: captureOccupied });
  const draft = autosave.draft;
  const [dictionaryEditing, setDictionaryEditing] = useState<string | null>(null);
  const state = useMemo(() => nativeState && autosave.saved ? { ...nativeState, settings: autosave.saved } : nativeState, [nativeState, autosave.saved]);
  const permissions = useSystemPermissions(native.available, 'keyboard-settings');
  const permission = permissions.status?.permissions.find(entry => entry.kind === 'microphone');
  const microphoneRef = useRef<HTMLDivElement>(null);
  const modelsRef = useRef<HTMLDivElement>(null);
  const deviceRef = useRef<HTMLDivElement>(null);
  const cloudRef = useRef<HTMLDivElement>(null);
  const visit = (target: { current: HTMLDivElement | null }) => { target.current?.scrollIntoView?.({ block: 'start', behavior: 'smooth' }); target.current?.focus({ preventScroll: true }); };
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
  const asrModels = asrMode === 'platform' ? catalog?.platformAsr ?? [] : catalog?.asr ?? [];
  const change = <K extends keyof Settings>(key: K, value: Settings[K], mode: 'immediate' | 'debounced' = 'immediate') => {
    autosave.change({ [key]: value } as Partial<Settings>, mode);
  };
  const changeAsrMode = (mode: KeyboardAsrMode) => {
    if (!draft) return;
    const models = mode === 'platform' ? catalog?.platformAsr ?? [] : catalog?.asr ?? [];
    autosave.change({ asrMode: mode,
      asrModel: models.some(model => modelSelectionKey(model) === modelSelectionKey(draft.asrModel)) ? draft.asrModel : null });
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

  return <SettingsPageWrapper>
    <PageHeader title={t('settings.keyboard.title')} actions={<Button size='small' loading={native.busy}
      onClick={() => { void native.refresh(); void refreshCatalog(); void refreshAudioDevices(); void permissions.refresh(); }} disabled={!native.available}>{t('settings.keyboard.refresh')}</Button>} />
    <div className='cv-stack'>
      {!native.available && <Alert type='info' content={t('settings.keyboard.desktopOnly')} />}
      {native.error && <Alert type='error' content={native.error} />}
      {native.available && !state && !native.error && <div className='keyboard-loading' role='status'><Spin />{t('settings.keyboard.loading')}</div>}
      {state && draft && readiness && <>
        <section role='status' aria-live='polite' aria-label={t('settings.keyboard.saveStatus')} className='keyboard-save-status'>
          <p>{t(autosave.status === 'failed' ? 'settings.keyboard.autosaveFailed' : autosave.deferred ? 'settings.keyboard.autosaveDeferred'
            : autosave.status === 'saving' ? 'settings.keyboard.autosaveSaving' : autosave.status === 'pending' ? 'settings.keyboard.autosavePending'
            : autosave.status === 'saved' ? 'settings.keyboard.autosaveSaved' : 'settings.keyboard.autosaveIdle')}</p>
          {autosave.error && <p>{autosave.error}</p>}
          {autosave.status === 'failed' && <Button size='small' onClick={() => { void autosave.retry(); }}>{t('settings.keyboard.retrySave')}</Button>}
        </section>
        {native.busy && <p className='cv-note' role='status'>{t('settings.keyboard.block_busy')}</p>}
        <VisualPanel title={t('settings.keyboard.setupTitle')} description={t('settings.keyboard.setupHint')}>
          <ol className='keyboard-setup-list'>
            <li><VisualRow label={t('settings.keyboard.setupHardware')} description={t(readiness.hardwareReady ? 'settings.keyboard.hardwareReady' : 'settings.keyboard.hardwarePending')}>
              <Button size='small' onClick={() => visit(deviceRef)}>{t('settings.keyboard.checkKeyboard')}</Button>
            </VisualRow></li>
            <li><VisualRow label={t('settings.keyboard.setupMicrophone')} description={t(readiness.permissionBlocked ? 'settings.keyboard.block_permission' : !readiness.deviceAvailable ? 'settings.keyboard.block_device' : localVerified ? 'settings.keyboard.localPassed' : 'settings.keyboard.localUnverified')}>
              <Button size='small' onClick={() => visit(microphoneRef)}>{t('settings.keyboard.checkMicrophone')}</Button>
            </VisualRow></li>
            <li><VisualRow label={t('settings.keyboard.setupModel')} description={t(readiness.savedModelAvailable ? 'settings.keyboard.modelReady' : 'settings.keyboard.block_model')}>
              <Button size='small' onClick={() => visit(modelsRef)}>{t('settings.keyboard.checkModels')}</Button>
            </VisualRow></li>
            <li><VisualRow label={t('settings.keyboard.setupCloud')} description={t(cloudResult.state === 'completed' ? 'settings.keyboard.cloudPassed' : 'settings.keyboard.cloudUnverified')}>
              <Button size='small' onClick={() => visit(cloudRef)}>{t('settings.keyboard.checkCloud')}</Button>
            </VisualRow></li>
          </ol>
        </VisualPanel>
        {state.error && <VisualPanel title={t('settings.keyboard.hardwareErrorTitle')}>
          <Alert type='warning' content={state.error} />
        </VisualPanel>}
        <div ref={deviceRef} tabIndex={-1} className='keyboard-section-target'>
        <VisualPanel title={t('settings.keyboard.deviceTitle')} description={t('settings.keyboard.deviceHint')}>
          <VisualRow label={t('settings.keyboard.connection')} description={[state.device.id, state.device.port].filter(Boolean).join(' · ')}>
            <VisualStatus tone={state.device.connected ? 'success' : 'warning'}>{t(state.device.connected ? 'settings.keyboard.connected' : 'settings.keyboard.disconnected')}</VisualStatus>
          </VisualRow>
          <VisualRow label={t('settings.keyboard.firmware')} description={state.device.firmware || t('settings.keyboard.unknown')} />
          <VisualRow label={t('settings.keyboard.runtimeStatus')} description={state.service.detail}>
            <VisualStatus tone={state.service.ready ? 'success' : 'warning'}>{t(state.service.ready ? 'settings.keyboard.ready' : state.service.running ? 'settings.keyboard.waiting' : 'settings.keyboard.stopped')}</VisualStatus>
          </VisualRow>
        </VisualPanel>
        <VisualPanel title={t('settings.keyboard.keysTitle')} description={t('settings.keyboard.keysHint')}>
          <VisualRow label={t('settings.keyboard.enabled')} description={t('settings.keyboard.enabledHint')}>
            <VisualSwitch label={t('settings.keyboard.enabled')} checked={draft.enabled} onChange={(value) => change('enabled', value)} />
          </VisualRow>
          {(['k2Enabled', 'k3Enabled', 'k4Enabled'] as const).map((key) => <VisualRow key={key} label={t(`settings.keyboard.${key}`)} disabled={!draft.enabled}>
            <VisualSwitch label={t(`settings.keyboard.${key}`)} checked={draft[key]} onChange={(value) => change(key, value)} />
          </VisualRow>)}
          <p className='cv-note'>{t('settings.keyboard.disableHint')}</p>
        </VisualPanel>
        </div>
        <div ref={microphoneRef} tabIndex={-1} className='keyboard-section-target'>
        <VisualPanel title={t('settings.keyboard.microphoneTitle')} description={t('settings.keyboard.microphoneHint')}
          action={<Button size='small' loading={audioDevicesLoading} onClick={() => void refreshAudioDevices()}>{t('settings.keyboard.refreshDevices')}</Button>}>
          {audioDevicesError && <Alert type='error' content={String(audioDevicesError)} />}
          <VisualRow label={t('settings.keyboard.permissionTitle')} description={t(`settings.keyboard.permission_${permission?.state ?? 'unknown'}`)}>
            <div className='keyboard-inline-actions'>
              {permission?.can_request && <Button size='small' disabled={permissions.loading} onClick={() => void permissions.request('microphone')}>{t('settings.keyboard.requestMicrophone')}</Button>}
              {permission?.can_open_settings && <Button size='small' disabled={permissions.loading} onClick={() => void permissions.openSettings('microphone')}>{t('settings.keyboard.openMicrophoneSettings')}</Button>}
            </div>
          </VisualRow>
          {permissions.loading && <p className='cv-note' role='status'>{t('settings.keyboard.permissionLoading')}</p>}
          {permissions.error && <Alert type='warning' content={permissions.error} />}
          {audioDevicesLoading && <p className='cv-note'>{t('settings.keyboard.inputsLoading')}</p>}
          {recognitionBusy && <p className='cv-note'>{t('settings.keyboard.recognitionEditingBusy')}</p>}
          <VisualRow label={t('settings.keyboard.audioDevice')}>
            <Select className='keyboard-model-select' aria-label={t('settings.keyboard.audioDevice')} value={draft.audioDeviceId ?? ''}
              disabled={recognitionBusy || audioDevicesLoading} onChange={(value: string) => change('audioDeviceId', value || null)}>
              <Select.Option value=''>{t('settings.keyboard.systemDefault')}</Select.Option>
              {draft.audioDeviceId && !audioDevices?.some((device) => device.id === draft.audioDeviceId) &&
                <Select.Option value={draft.audioDeviceId} disabled>{t('settings.keyboard.unavailableAudioDevice')}</Select.Option>}
              {(audioDevices ?? []).map((device) => <Select.Option key={device.id} value={device.id}>
                {device.label}{device.isDefault ? ` · ${t('settings.keyboard.defaultDevice')}` : ''}
              </Select.Option>)}
            </Select>
          </VisualRow>
          <VisualRow label={t('settings.keyboard.microphoneState')} description={state.audioState.detail}>
            <VisualStatus tone={state.audioState.state === 'error' ? 'danger' : state.audioState.state === 'stopped' ? 'neutral' : 'info'}>
              {t(`settings.keyboard.audio_${state.audioState.state}`)}
            </VisualStatus>
          </VisualRow>
          <p className='cv-note'>{t('settings.keyboard.microphonePermission')}</p>
          <section className='keyboard-test-section' aria-label={t('settings.keyboard.localTitle')}>
            <h3>{t('settings.keyboard.localTitle')}</h3>
            <p className='cv-note'>{t('settings.keyboard.localHint')}</p>
            <div className='keyboard-form-actions'>
              <Button disabled={localBlocked.length > 0} onClick={() => void local.start()}>{t('settings.keyboard.localStart')}</Button>
              <Button disabled={!local.active || local.result.state === 'stopping'} onClick={() => void local.stop()}>{t('settings.keyboard.localStop')}</Button>
            </div>
            {localBlocked.length > 0 && <ul className='keyboard-blockers'>{localBlocked.map(reason => <li key={reason}>{blockerText(reason)}</li>)}</ul>}
            {!local.active && <p className='cv-note'>{t('settings.keyboard.localStopIdle')}</p>}
            <div role='status' aria-live='polite'><p>{t(`settings.keyboard.${localMessage}`)}</p>
              {local.result.detail && <p>{local.result.detail}</p>}
              {local.result.callbackObserved && <p>{t('settings.keyboard.localCallback')}</p>}
            </div>
            {local.result.id && <div className='keyboard-meter'>
              <meter aria-label={t('settings.keyboard.localLevel')} min={0} max={1} value={Math.min(1, Math.max(0, local.result.peak))} />
              <span>{t('settings.keyboard.localStats', { seconds: (local.result.elapsedMs / 1000).toFixed(1), samples: local.result.samples })}</span>
            </div>}
          </section>
        </VisualPanel>
        </div>
        <div ref={modelsRef} tabIndex={-1} className='keyboard-section-target'>
        <VisualPanel title={t('settings.keyboard.modelsTitle')} description={t('settings.keyboard.modelsHint')}
          action={<Link to='/models'>{t('settings.keyboard.manageModels')}</Link>}>
          {catalogError && <Alert type='error' content={String(catalogError)} />}
          {recognitionBusy && <p className='cv-note'>{t('settings.keyboard.recognitionEditingBusy')}</p>}
          {(catalogLoading || !catalog) && <p className='cv-note'>{t('settings.keyboard.catalogLoading')}</p>}
          {readiness.modelCatalogueEmpty && <Alert type='info' content={t(asrMode === 'platform' ? 'settings.keyboard.emptyPlatform' : 'settings.keyboard.emptyStreaming')} />}
          {readiness.platformAlternative && <p className='cv-note'>{t('settings.keyboard.platformAlternative', { mode: t('settings.keyboard.asrModePlatform') })}</p>}
          <VisualRow label={t('settings.keyboard.asrMode')} description={t('settings.keyboard.asrModeHint')}>
            <Select className='keyboard-model-select' aria-label={t('settings.keyboard.asrMode')} value={asrMode}
              disabled={recognitionBusy || catalogLoading || !catalog} onChange={changeAsrMode}>
              <Select.Option value='streaming'>{t('settings.keyboard.asrModeStreaming')}</Select.Option>
              <Select.Option value='platform'>{t('settings.keyboard.asrModePlatform')}</Select.Option>
            </Select>
          </VisualRow>
          <VisualRow label={t('settings.keyboard.asrModel')} description={t(asrMode === 'platform' ? 'settings.keyboard.asrHintPlatform' : 'settings.keyboard.asrHint')}>
            <ModelSelect models={asrModels} value={draft.asrModel} label={t('settings.keyboard.asrModel')}
              disabled={recognitionBusy || catalogLoading || !catalog} onChange={(value) => change('asrModel', value)} />
          </VisualRow>
          <VisualRow label={t('settings.keyboard.language')} description={t('settings.keyboard.languageHint')}>
            <Input aria-label={t('settings.keyboard.language')} value={draft.language} disabled={recognitionBusy} {...textEvents('language')} onChange={(value) => change('language', value, 'debounced')} className='keyboard-short-input' />
          </VisualRow>
          <label className='keyboard-field'><span>{t('settings.keyboard.dictionary')}</span>
            <Input.TextArea value={dictionaryEditing ?? draft.dictionary.join('\n')} disabled={recognitionBusy} {...textEvents('dictionary')}
              onFocus={() => setDictionaryEditing(draft.dictionary.join('\n'))}
              onChange={(value) => { setDictionaryEditing(value); change('dictionary', value.split('\n'), 'debounced'); }}
              autoSize={{ minRows: 3, maxRows: 7 }} aria-label={t('settings.keyboard.dictionary')} placeholder={t('settings.keyboard.dictionaryHint')} />
          </label>
          <VisualRow label={t('settings.keyboard.textModel')}>
            <ModelSelect models={catalog?.text ?? []} value={draft.textModel} label={t('settings.keyboard.textModel')}
              disabled={catalogLoading || !catalog} onChange={(value) => change('textModel', value)} />
          </VisualRow>
          <label className='keyboard-field'><span>{t('settings.keyboard.instruction')}</span>
            <Input.TextArea value={draft.beautifyInstruction} {...textEvents('beautifyInstruction')} onChange={(value) => change('beautifyInstruction', value, 'debounced')}
              autoSize={{ minRows: 3, maxRows: 8 }} aria-label={t('settings.keyboard.instruction')} />
            <small>{t('settings.keyboard.instructionHint')}</small>
          </label>
          <p className='cv-note'>{t('settings.keyboard.textModelOptional')}</p>
          <section ref={cloudRef} tabIndex={-1} className='keyboard-test-section keyboard-section-target' aria-label={t('settings.keyboard.cloudTitle')}>
            <h3>{t('settings.keyboard.cloudTitle')}</h3>
            <p className='cv-note'>{t(asrMode === 'platform' ? 'settings.keyboard.testHintPlatform' : 'settings.keyboard.testHint')}</p>
            <div className='keyboard-form-actions'>
              <Button disabled={cloudBlocked.length > 0} onClick={() => void startTest()}>{t('settings.keyboard.testStart')}</Button>
              <Button disabled={!state.capturePreparing && state.phase !== 'recording' && !['starting', 'capturing'].includes(state.audioState.state)} onClick={() => void native.action({ action: 'test_stop', documentId: state.documentId, revision: state.revision })}>{t('settings.keyboard.testStop')}</Button>
            </div>
            {cloudBlocked.length > 0 && <ul className='keyboard-blockers'>{cloudBlocked.map(reason => <li key={reason}>{blockerText(reason)}</li>)}</ul>}
            {!captureOccupied && <p className='cv-note'>{t('settings.keyboard.cloudStopIdle')}</p>}
            <div role='status' aria-live='polite'>
              <p>{t(cloudResult.state === 'completed' ? 'settings.keyboard.cloudPassed' : cloudResult.state === 'running' ? 'settings.keyboard.cloudRunning' : cloudResult.state === 'failed' ? 'settings.keyboard.cloudFailed' : cloudResult.state === 'cancelled' ? 'settings.keyboard.cloudCancelled' : 'settings.keyboard.cloudUnverified')}</p>
              {cloudResult.detail && <p>{cloudResult.detail}</p>}
            </div>
          </section>
        </VisualPanel>
        </div>
        <VisualPanel title={t('settings.keyboard.windowTitle')}>
          <VisualRow label={t('settings.keyboard.subtitleFontSize')} description={t('settings.keyboard.subtitleFontSizeHint')}>
            <Select className='keyboard-font-select' aria-label={t('settings.keyboard.subtitleFontSize')} value={draft.subtitleFontSize}
              onChange={(value: number) => change('subtitleFontSize', value)}>
              {Array.from({ length: 21 }, (_, index) => index + 12).map(size => <Select.Option key={size} value={size}>{size}px</Select.Option>)}
            </Select>
          </VisualRow>
          <VisualRow label={t('settings.keyboard.alwaysOnTop')}>
            <VisualSwitch label={t('settings.keyboard.alwaysOnTop')} checked={draft.alwaysOnTop} onChange={(value) => change('alwaysOnTop', value)} />
          </VisualRow>
          <div className='keyboard-form-actions'>
            <Button disabled={native.busy} onClick={() => void native.action({ action: 'open' })}>{t('settings.keyboard.open')}</Button>
            <Button disabled={native.busy} onClick={() => void native.action({ action: 'reset_position' })}>{t('settings.keyboard.resetPosition')}</Button>
          </div>
        </VisualPanel>

      </>}
    </div>
  </SettingsPageWrapper>;
}
