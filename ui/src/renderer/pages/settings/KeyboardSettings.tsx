import { Alert, Button, Input, Select, Spin } from '@arco-design/web-react';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Link } from 'react-router-dom';
import useSWR from 'swr';
import { keyboard, modelSelectionKey, parseModelSelection, type KeyboardAsrMode, type KeyboardCatalogModel, type KeyboardModel, type KeyboardSettings as Settings, type KeyboardState } from '@/common/keyboard';
import { useKeyboard } from '@/renderer/hooks/keyboard/useKeyboard';
import PageHeader from '@/renderer/components/layout/PageHeader';
import { VisualPanel, VisualRow, VisualStatus, VisualSwitch } from './components/CodeVisualPrimitives';
import SettingsPageWrapper from './components/SettingsPageWrapper';
import './keyboard-settings.css';

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
  const state = native.state;
  const { data: audioDevices, error: audioDevicesError, isValidating: audioDevicesLoading, mutate: refreshAudioDevices } = useSWR(
    native.available ? 'keyboard-audio-devices' : null, keyboard.getAudioDevices,
    { revalidateOnFocus: false, shouldRetryOnError: false }
  );
  const [draft, setDraft] = useState<Settings | null>(null);
  const [dirty, setDirty] = useState(false);
  const [testError, setTestError] = useState<string | null>(null);
  const test = useRef<{ sequence: number; documentId: string | null; operationId: string | null; active: boolean; acknowledged: boolean } | null>(null);
  const observeTest = (next: KeyboardState) => {
    const request = test.current;
    if (!request || next.sequence <= request.sequence) return;
    if (request.operationId && next.operationId && next.operationId !== request.operationId
      || request.documentId && next.documentId && next.documentId !== request.documentId) {
      // A later hardware operation owns this state; never replay its failure
      // as the result of an earlier settings-page microphone test.
      test.current = null;
      return;
    }
    const active = next.capturePreparing || ['recording', 'finalizing'].includes(next.phase)
      || ['starting', 'capturing', 'stopping'].includes(next.audioState.state);
    if (active) {
      request.active = true;
      request.documentId ??= next.documentId;
      request.operationId ??= next.operationId;
    } else if (request.active || request.acknowledged) {
      setTestError(next.error);
      test.current = null;
    }
  };
  useEffect(() => { if (state) observeTest(state); }, [state]);
  const startTest = async () => {
    const baseline = native.getCurrentState();
    if (!baseline) return;
    setTestError(null);
    const request = { sequence: baseline.sequence, documentId: null, operationId: null, active: false, acknowledged: false };
    test.current = request;
    const next = await native.action({ action: 'test_start' });
    if (test.current !== request) return;
    request.acknowledged = true;
    if (next) observeTest(native.getCurrentState() ?? next);
    else test.current = null;
  };
  const asrMode = draft?.asrMode ?? 'streaming';
  const asrModels = asrMode === 'platform' ? catalog?.platformAsr ?? [] : catalog?.asr ?? [];
  useEffect(() => {
    if (state && !dirty) setDraft(state.settings);
  }, [state, dirty]);

  const change = <K extends keyof Settings>(key: K, value: Settings[K]) => {
    setDraft((current) => current ? { ...current, [key]: value } : current);
    setDirty(true);
  };
  const changeAsrMode = (mode: KeyboardAsrMode) => {
    const models = mode === 'platform' ? catalog?.platformAsr ?? [] : catalog?.asr ?? [];
    setDraft((current) => current ? { ...current, asrMode: mode,
      asrModel: models.some((model) => modelSelectionKey(model) === modelSelectionKey(current.asrModel)) ? current.asrModel : null,
    } : current);
    setDirty(true);
  };
  const toggle = async (key: 'enabled' | 'k2Enabled' | 'k3Enabled' | 'k4Enabled' | 'alwaysOnTop', value: boolean) => {
    if (!state) return;
    const updated = await native.updateSettings({ ...state.settings, [key]: value });
    if (updated) setDraft((current) => current ? { ...current, [key]: updated.settings[key] } : updated.settings);
  };
  const save = async () => {
    if (!draft || !state) return;
    const updated = await native.updateSettings({ ...draft,
      enabled: state.settings.enabled, k2Enabled: state.settings.k2Enabled,
      k3Enabled: state.settings.k3Enabled, k4Enabled: state.settings.k4Enabled,
      alwaysOnTop: state.settings.alwaysOnTop,
    });
    if (updated) { setDraft(updated.settings); setDirty(false); }
  };

  return <SettingsPageWrapper>
    <PageHeader title={t('settings.keyboard.title')} actions={<Button size='small' loading={native.busy}
      onClick={() => { void native.refresh(); void refreshCatalog(); void refreshAudioDevices(); }} disabled={!native.available}>{t('settings.keyboard.refresh')}</Button>} />
    <div className='cv-stack'>
      {!native.available && <Alert type='info' content={t('settings.keyboard.desktopOnly')} />}
      {native.error && <Alert type='error' content={native.error} />}
      {native.available && !state && !native.error && <div className='keyboard-loading' role='status'><Spin />{t('settings.keyboard.loading')}</div>}
      {state && draft && <>
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
            <VisualSwitch label={t('settings.keyboard.enabled')} checked={state.settings.enabled} disabled={native.busy} onChange={(value) => toggle('enabled', value)} />
          </VisualRow>
          {(['k2Enabled', 'k3Enabled', 'k4Enabled'] as const).map((key) => <VisualRow key={key} label={t(`settings.keyboard.${key}`)} disabled={!state.settings.enabled}>
            <VisualSwitch label={t(`settings.keyboard.${key}`)} checked={state.settings[key]} disabled={native.busy} onChange={(value) => toggle(key, value)} />
          </VisualRow>)}
          <p className='cv-note'>{t('settings.keyboard.disableHint')}</p>
        </VisualPanel>
        <VisualPanel title={t('settings.keyboard.microphoneTitle')} description={t('settings.keyboard.microphoneHint')}
          action={<Button size='small' loading={audioDevicesLoading} onClick={() => void refreshAudioDevices()}>{t('settings.keyboard.refreshDevices')}</Button>}>
          {audioDevicesError && <Alert type='error' content={String(audioDevicesError)} />}
          <VisualRow label={t('settings.keyboard.audioDevice')}>
            <Select className='keyboard-model-select' aria-label={t('settings.keyboard.audioDevice')} value={draft.audioDeviceId ?? ''}
              disabled={native.busy || audioDevicesLoading} onChange={(value: string) => change('audioDeviceId', value || null)}>
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
        </VisualPanel>
        <VisualPanel title={t('settings.keyboard.modelsTitle')} description={t('settings.keyboard.modelsHint')}
          action={<Link to='/models'>{t('settings.keyboard.manageModels')}</Link>}>
          {catalogError && <Alert type='error' content={String(catalogError)} />}
          {testError && <Alert type='error' content={testError} />}
          <VisualRow label={t('settings.keyboard.asrMode')} description={t('settings.keyboard.asrModeHint')}>
            <Select className='keyboard-model-select' aria-label={t('settings.keyboard.asrMode')} value={asrMode}
              disabled={native.busy || catalogLoading || !catalog} onChange={changeAsrMode}>
              <Select.Option value='streaming'>{t('settings.keyboard.asrModeStreaming')}</Select.Option>
              <Select.Option value='platform'>{t('settings.keyboard.asrModePlatform')}</Select.Option>
            </Select>
          </VisualRow>
          <VisualRow label={t('settings.keyboard.asrModel')} description={t(asrMode === 'platform' ? 'settings.keyboard.asrHintPlatform' : 'settings.keyboard.asrHint')}>
            <ModelSelect models={asrModels} value={draft.asrModel} label={t('settings.keyboard.asrModel')}
              disabled={native.busy || catalogLoading || !catalog} onChange={(value) => change('asrModel', value)} />
          </VisualRow>
          <VisualRow label={t('settings.keyboard.language')} description={t('settings.keyboard.languageHint')}>
            <Input aria-label={t('settings.keyboard.language')} value={draft.language} onChange={(value) => change('language', value)} className='keyboard-short-input' />
          </VisualRow>
          <label className='keyboard-field'><span>{t('settings.keyboard.dictionary')}</span>
            <Input.TextArea value={draft.dictionary.join('\n')} onChange={(value) => change('dictionary', value.split('\n'))}
              autoSize={{ minRows: 3, maxRows: 7 }} aria-label={t('settings.keyboard.dictionary')} placeholder={t('settings.keyboard.dictionaryHint')} />
          </label>
          <VisualRow label={t('settings.keyboard.textModel')}>
            <ModelSelect models={catalog?.text ?? []} value={draft.textModel} label={t('settings.keyboard.textModel')}
              disabled={native.busy || catalogLoading || !catalog} onChange={(value) => change('textModel', value)} />
          </VisualRow>
          <label className='keyboard-field'><span>{t('settings.keyboard.instruction')}</span>
            <Input.TextArea value={draft.beautifyInstruction} onChange={(value) => change('beautifyInstruction', value)}
              autoSize={{ minRows: 3, maxRows: 8 }} aria-label={t('settings.keyboard.instruction')} />
            <small>{t('settings.keyboard.instructionHint')}</small>
          </label>
          <div className='keyboard-form-actions'>
            <Button disabled={native.busy || dirty || state.capturePreparing || !state.settings.asrModel || state.phase !== 'idle' || !['stopped', 'error'].includes(state.audioState.state)}
              onClick={() => void startTest()}>{t('settings.keyboard.testStart')}</Button>
            <Button disabled={!state.capturePreparing && state.phase !== 'recording' && !['starting', 'capturing'].includes(state.audioState.state)} onClick={() => void native.action({ action: 'test_stop', documentId: state.documentId, revision: state.revision })}>{t('settings.keyboard.testStop')}</Button>
          </div>
          <p className='cv-note'>{t(asrMode === 'platform' ? 'settings.keyboard.testHintPlatform' : 'settings.keyboard.testHint')}</p>
        </VisualPanel>
        <VisualPanel title={t('settings.keyboard.windowTitle')}>
          <VisualRow label={t('settings.keyboard.subtitleFontSize')} description={t('settings.keyboard.subtitleFontSizeHint')}>
            <Select className='keyboard-font-select' aria-label={t('settings.keyboard.subtitleFontSize')} value={draft.subtitleFontSize}
              disabled={native.busy} onChange={(value: number) => change('subtitleFontSize', value)}>
              {Array.from({ length: 21 }, (_, index) => index + 12).map(size => <Select.Option key={size} value={size}>{size}px</Select.Option>)}
            </Select>
          </VisualRow>
          <VisualRow label={t('settings.keyboard.alwaysOnTop')}>
            <VisualSwitch label={t('settings.keyboard.alwaysOnTop')} checked={state.settings.alwaysOnTop} disabled={native.busy} onChange={(value) => toggle('alwaysOnTop', value)} />
          </VisualRow>
          <div className='keyboard-form-actions'>
            <Button disabled={native.busy} onClick={() => void native.action({ action: 'open' })}>{t('settings.keyboard.open')}</Button>
            <Button disabled={native.busy} onClick={() => void native.action({ action: 'reset_position' })}>{t('settings.keyboard.resetPosition')}</Button>
          </div>
        </VisualPanel>
        <div className='keyboard-form-actions'>
          <Button type='primary' loading={native.busy} disabled={!dirty} onClick={() => void save()}>{t('settings.keyboard.save')}</Button>
          <Button disabled={!dirty || native.busy} onClick={() => { setDraft(state.settings); setDirty(false); }}>{t('settings.keyboard.discard')}</Button>
        </div>
      </>}
    </div>
  </SettingsPageWrapper>;
}
