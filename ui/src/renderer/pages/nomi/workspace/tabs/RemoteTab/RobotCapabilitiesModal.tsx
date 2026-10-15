/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react';
import { Button, Modal, Spin, Tag } from '@arco-design/web-react';
import { useTranslation } from 'react-i18next';
import { useSWRConfig } from 'swr';
import { ipcBridge } from '@/common';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import type {
  IApiRobot,
  IApiRobotCapabilities,
  IApiRobotCapabilityAction,
  IApiRobotCapabilityOverride,
  IApiRobotCapabilityPreset,
  IApiRobotCapabilityProfile,
  IApiRobotCapabilityTool,
} from '@/common/adapter/ipcBridge';
import NomiModal from '@/renderer/components/base/NomiModal';
import type { I18nKey } from '@/renderer/services/i18n/i18n-keys';
import RobotCapabilityArguments from './RobotCapabilityArguments';
import styles from './RobotCapabilitiesModal.module.css';

const actionKeys: Record<IApiRobotCapabilityAction, I18nKey> = {
  vision: 'nomi.robot.capabilities.actions.vision',
  display: 'nomi.robot.capabilities.actions.display',
  motion: 'nomi.robot.capabilities.actions.motion',
  device: 'nomi.robot.capabilities.actions.device',
};
const overrideFor = (): IApiRobotCapabilityOverride => ({
  enabled: true, action: null, description: null, default_arguments: {},
});

interface ToolEditorProps {
  tool: IApiRobotCapabilityTool;
  override?: IApiRobotCapabilityOverride;
  onChange: (next: IApiRobotCapabilityOverride | null) => void;
  onValidityChange: (key: string, valid: boolean) => void;
}

function ToolEditor({ tool, override, onChange, onValidityChange }: ToolEditorProps) {
  const { t } = useTranslation();
  const id = useId();
  const current = override ?? overrideFor();
  const update = (patch: Partial<IApiRobotCapabilityOverride>) => onChange({ ...current, ...patch });
  const validity = useCallback((key: string, valid: boolean) =>
    onValidityChange(`tool:${tool.device_name}:${key}`, valid), [onValidityChange, tool.device_name]);
  const action = current.action ?? tool.action;
  return <details className={styles.card}>
    <summary>
      <span className={styles.toolHeader}>
        <code>{tool.device_name}</code>
        <Tag size='small'>{t(actionKeys[action])}</Tag>
        {!current.enabled && <span className={styles.muted}>{t('nomi.robot.capabilities.disabled')}</span>}
      </span>
    </summary>
    <div className={styles.fields}>
      <p className={styles.hint}>{tool.description || t('nomi.robot.capabilities.noDescription')}</p>
      <div className={styles.toolbar}>
        <label className={styles.inlineLabel}>
          <input type='checkbox' checked={current.enabled} onChange={(event) => update({ enabled: event.target.checked })} />
          {t('nomi.robot.capabilities.enabled')}
        </label>
        <Button size='mini' disabled={!override} onClick={() => onChange(null)}>
          {t('nomi.robot.capabilities.restoreDefault')}
        </Button>
      </div>
      <div className={styles.field}>
        <label htmlFor={`${id}-action`}>{t('nomi.robot.capabilities.action')}</label>
        <select id={`${id}-action`} className={styles.input} value={current.action ?? ''}
          onChange={(event) => update({ action: (event.target.value || null) as IApiRobotCapabilityAction | null })}>
          <option value=''>{t('nomi.robot.capabilities.defaultAction')}</option>
          {tool.allowed_actions.map((item) => <option key={item} value={item}>{t(actionKeys[item])}</option>)}
        </select>
      </div>
      <div className={styles.field}>
        <label htmlFor={`${id}-description`}>{t('nomi.robot.capabilities.description')}</label>
        <textarea id={`${id}-description`} className={styles.input} value={current.description ?? ''}
          placeholder={tool.description} rows={2}
          onChange={(event) => update({ description: event.target.value || null })} />
      </div>
      <div className={styles.section}>
        <h4>{t('nomi.robot.capabilities.defaultArguments')}</h4>
        <p className={styles.hint}>{t('nomi.robot.capabilities.defaultArgumentsHint')}</p>
        <RobotCapabilityArguments schema={tool.input_schema} value={current.default_arguments}
          onChange={(default_arguments) => update({ default_arguments })} onValidityChange={validity} />
      </div>
      <details>
        <summary>{t('nomi.robot.capabilities.rawSchema')}</summary>
        <pre className={styles.schema}>{JSON.stringify(tool.input_schema, null, 2)}</pre>
        <p className={styles.hint}>{t('nomi.robot.capabilities.exposedName', { name: tool.exposed_name })}</p>
      </details>
    </div>
  </details>;
}

interface PresetEditorProps {
  preset: IApiRobotCapabilityPreset;
  rowKey: string;
  tools: IApiRobotCapabilityTool[];
  onChange: (next: IApiRobotCapabilityPreset) => void;
  onRemove: () => void;
  onValidityChange: (key: string, valid: boolean) => void;
}

function PresetEditor({ preset, rowKey, tools, onChange, onRemove, onValidityChange }: PresetEditorProps) {
  const { t } = useTranslation();
  const id = useId();
  const tool = tools.find((item) => item.device_name === preset.tool_name);
  const update = (patch: Partial<IApiRobotCapabilityPreset>) => onChange({ ...preset, ...patch });
  const validity = useCallback((key: string, valid: boolean) =>
    onValidityChange(`preset:${rowKey}:${key}`, valid), [onValidityChange, rowKey]);
  return <details className={styles.card} open>
    <summary>{preset.name || preset.id}</summary>
    <div className={styles.fields}>
      <div className={styles.toolbar}>
        <label className={styles.inlineLabel}>
          <input type='checkbox' checked={preset.enabled} onChange={(event) => update({ enabled: event.target.checked })} />
          {t('nomi.robot.capabilities.enabled')}
        </label>
        <Button size='mini' status='danger' onClick={onRemove}>{t('nomi.robot.capabilities.removePreset')}</Button>
      </div>
      <div className={styles.field}>
        <label htmlFor={`${id}-name`}>{t('nomi.robot.capabilities.presetName')}</label>
        <input id={`${id}-name`} className={styles.input} value={preset.name}
          onChange={(event) => update({ name: event.target.value })} />
      </div>
      <div className={styles.field}>
        <label htmlFor={`${id}-id`}>{t('nomi.robot.capabilities.presetId')}</label>
        <input id={`${id}-id`} className={styles.input} value={preset.id} maxLength={32}
          pattern='[a-z][a-z0-9_]{0,31}' onChange={(event) => update({ id: event.target.value })} />
        <span className={styles.hint}>{t('nomi.robot.capabilities.presetIdHint')}</span>
      </div>
      <div className={styles.field}>
        <label htmlFor={`${id}-tool`}>{t('nomi.robot.capabilities.presetTool')}</label>
        <select id={`${id}-tool`} className={styles.input} value={preset.tool_name}
          onChange={(event) => update({ tool_name: event.target.value, arguments: {} })}>
          {!tool && <option value={preset.tool_name}>{preset.tool_name || t('nomi.robot.capabilities.chooseTool')}</option>}
          {tools.map((item) => <option key={item.device_name} value={item.device_name}>{item.device_name}</option>)}
        </select>
        {!tool && <span className={styles.error}>{t('nomi.robot.capabilities.toolUnavailable')}</span>}
      </div>
      <div className={styles.field}>
        <label htmlFor={`${id}-description`}>{t('nomi.robot.capabilities.presetDescription')}</label>
        <textarea id={`${id}-description`} className={styles.input} value={preset.description} rows={2}
          onChange={(event) => update({ description: event.target.value })} />
      </div>
      <div className={styles.section}>
        <h4>{t('nomi.robot.capabilities.presetArguments')}</h4>
        <RobotCapabilityArguments key={preset.tool_name} schema={tool?.input_schema ?? {}} value={preset.arguments}
          onChange={(args) => update({ arguments: args })} onValidityChange={validity} />
      </div>
    </div>
  </details>;
}

interface RobotCapabilitiesModalProps {
  robot: IApiRobot;
  onCancel: () => void;
  onSaved?: () => void;
}

export default function RobotCapabilitiesModal({ robot, onCancel, onSaved }: RobotCapabilitiesModalProps) {
  const { t } = useTranslation();
  const { mutate } = useSWRConfig();
  const [snapshot, setSnapshot] = useState<IApiRobotCapabilities | null>(null);
  const [draft, setDraft] = useState<IApiRobotCapabilityProfile | null>(null);
  const [presetKeys, setPresetKeys] = useState<string[]>([]);
  const presetSequence = useRef(0);
  const loadedRobotId = useRef<string | null>(null);
  const nextPresetKey = useCallback(() => `preset-row-${++presetSequence.current}`, []);
  const [busy, setBusy] = useState<'load' | 'save' | 'refresh' | null>('load');
  const [error, setError] = useState('');
  const [conflict, setConflict] = useState(false);
  const [saved, setSaved] = useState(false);
  const [invalidFields, setInvalidFields] = useState<Set<string>>(new Set());
  const dirty = snapshot != null && JSON.stringify(snapshot.profile) !== JSON.stringify(draft);
  const validity = useCallback((key: string, valid: boolean) => setInvalidFields((previous) => {
    if (previous.has(key) === !valid) return previous;
    const next = new Set(previous);
    if (valid) next.delete(key); else next.add(key);
    return next;
  }), []);
  const accept = useCallback((next: IApiRobotCapabilities) => {
    setSnapshot(next);
    setDraft(structuredClone(next.profile));
    setPresetKeys(next.profile.presets.map(nextPresetKey));
    setInvalidFields(new Set());
    setConflict(false);
    setError('');
  }, [nextPresetKey]);
  const errorText = (caught: unknown, fallback: I18nKey) =>
    isBackendHttpError(caught) && caught.backendMessage ? caught.backendMessage : t(fallback);
  const load = useCallback(async () => {
    setBusy('load');
    try {
      accept(await ipcBridge.robot.getCapabilities.invoke({ robot_id: robot.robot_id }));
      setSaved(false);
    } catch (caught) {
      setError(isBackendHttpError(caught) && caught.backendMessage ? caught.backendMessage : t('nomi.robot.capabilities.loadFailed'));
    } finally { setBusy(null); }
  }, [accept, robot.robot_id, t]);
  useEffect(() => {
    if (loadedRobotId.current === robot.robot_id) return;
    loadedRobotId.current = robot.robot_id;
    void load();
  }, [load, robot.robot_id]);

  const confirmDiscard = (action: () => void) => {
    if (!dirty && invalidFields.size === 0) { action(); return; }
    Modal.confirm({ title: t('nomi.robot.capabilities.discardTitle'), content: t('nomi.robot.capabilities.discardHint'),
      onOk: action });
  };
  const notifyResources = async () => {
    try {
      await mutate((key) => key === 'robots.list' ||
        (typeof key === 'string' && key.startsWith('companion-robot-resources')) ||
        (Array.isArray(key) && typeof key[0] === 'string' && key[0].startsWith('companion-robot-resources')));
    } catch (caught) {
      console.warn('[RobotCapabilities] Could not refresh conversation resource options:', caught);
    }
    onSaved?.();
  };
  const save = async () => {
    if (!snapshot || !draft || !snapshot.connected || busy) return;
    setBusy('save'); setError('');
    try {
      const next = await ipcBridge.robot.setCapabilities.invoke({ robot_id: robot.robot_id,
        expected_revision: snapshot.revision, profile: draft });
      accept(next); setSaved(true);
      await notifyResources();
    } catch (caught) {
      if (isBackendHttpError(caught) && caught.status === 409 &&
        (caught.code === '' || caught.code === 'ROBOT_CAPABILITIES_CHANGED')) setConflict(true);
      else if (isBackendHttpError(caught) && caught.code === 'ROBOT_OFFLINE') {
        setError(t('nomi.robot.capabilities.offline'));
      } else setError(errorText(caught, 'nomi.robot.capabilities.saveFailed'));
    } finally { setBusy(null); }
  };
  const refresh = async () => {
    if (!snapshot?.connected || busy || dirty || invalidFields.size > 0) return;
    setBusy('refresh'); setError('');
    try {
      accept(await ipcBridge.robot.refreshCapabilities.invoke({ robot_id: robot.robot_id }));
      setSaved(true);
      await notifyResources();
    } catch (caught) { setError(errorText(caught, 'nomi.robot.capabilities.refreshFailed')); }
    finally { setBusy(null); }
  };
  const presetError = useMemo(() => {
    if (!draft) return false;
    const ids = draft.presets.map((preset) => preset.id);
    return new Set(ids).size !== ids.length || draft.presets.some((preset) =>
      !/^[a-z][a-z0-9_]{0,31}$/.test(preset.id) || !preset.name.trim() ||
      (!snapshot?.tools.some((tool) => tool.device_name === preset.tool_name) &&
        !snapshot?.profile.presets.some((savedPreset) => JSON.stringify(savedPreset) === JSON.stringify(preset))));
  }, [draft, snapshot]);
  const addPreset = () => {
    if (!draft || !snapshot?.tools[0]) return;
    let number = draft.presets.length + 1;
    while (draft.presets.some((preset) => preset.id === `preset_${number}`)) number++;
    setDraft({ ...draft, presets: [...draft.presets, {
      id: `preset_${number}`, name: t('nomi.robot.capabilities.newPreset'),
      tool_name: snapshot.tools[0].device_name, description: '', arguments: {}, enabled: true,
    }] });
    setPresetKeys((keys) => [...keys, nextPresetKey()]);
    setSaved(false);
  };
  return <NomiModal visible header={{ title: t('nomi.robot.capabilities.title', { name: robot.name }), showClose: true }}
    style={{ width: 800, maxWidth: 'calc(100vw - 48px)' }} contentStyle={{ maxHeight: '66vh', overflow: 'auto' }}
    onCancel={() => { if (!busy) confirmDiscard(onCancel); }} maskClosable={false}
    footer={<div className={styles.footer}>
      <Button disabled={busy != null} onClick={() => confirmDiscard(onCancel)}>{t('common.close')}</Button>
      <Button type='primary' loading={busy === 'save'}
        disabled={!snapshot?.connected || busy != null || !dirty || conflict || invalidFields.size > 0 || presetError}
        onClick={() => void save()}>{t('nomi.robot.capabilities.save')}</Button>
    </div>}>
    <div className={styles.body}>
      <p className={styles.hint}>{t('nomi.robot.capabilities.hint')}</p>
      <div className={styles.notice}>{t('nomi.robot.capabilities.agentHint')}</div>
      {busy === 'load' && <Spin />}
      {error && <div role='alert' className={styles.error}>{error}</div>}
      {conflict && <div role='alert' className={styles.notice}>{t('nomi.robot.capabilities.conflict')}</div>}
      {saved && <div role='status' className={styles.notice}>{t('nomi.robot.capabilities.saved')}</div>}
      {snapshot && draft && <>
        {!snapshot.connected && <div className={styles.notice}>{t('nomi.robot.capabilities.offline')}</div>}
        <div className={styles.toolbar}>
          <span className={styles.hint}>{t('nomi.robot.capabilities.revision', { revision: snapshot.revision })}</span>
          <div className={styles.toolbar}>
            <Button size='small' disabled={busy != null} onClick={() => confirmDiscard(() => void load())}>
              {t('nomi.robot.capabilities.reload')}
            </Button>
            <Button size='small' loading={busy === 'refresh'} disabled={!snapshot.connected || busy != null || dirty || invalidFields.size > 0}
              onClick={() => void refresh()}>{t('nomi.robot.capabilities.refresh')}</Button>
          </div>
        </div>
        {(dirty || invalidFields.size > 0) && <p className={styles.hint}>{t('nomi.robot.capabilities.draftHint')}</p>}
        <fieldset className={styles.fieldset} disabled={!snapshot.connected || busy != null}>
          <div className={styles.body}>
            <section className={styles.section}>
              <h4>{t('nomi.robot.capabilities.tools')}</h4>
              {snapshot.tools.length === 0 && <p className={styles.hint}>{t('nomi.robot.capabilities.noTools')}</p>}
              {snapshot.tools.map((tool) => <ToolEditor key={tool.device_name} tool={tool}
                override={draft.tool_overrides[tool.device_name]} onValidityChange={validity}
                onChange={(next) => {
                  const tool_overrides = { ...draft.tool_overrides };
                  if (next) tool_overrides[tool.device_name] = next;
                  else delete tool_overrides[tool.device_name];
                  setDraft({ ...draft, tool_overrides }); setSaved(false);
                }} />)}
              {Object.keys(draft.tool_overrides).filter((name) => !snapshot.tools.some((tool) => tool.device_name === name)).map((name) =>
                <div key={name} className={styles.card}>
                  <code>{name}</code><p className={styles.hint}>{t('nomi.robot.capabilities.toolUnavailable')}</p>
                  <pre className={styles.schema}>{JSON.stringify(draft.tool_overrides[name], null, 2)}</pre>
                  <Button size='mini' onClick={() => {
                    const tool_overrides = { ...draft.tool_overrides }; delete tool_overrides[name];
                    setDraft({ ...draft, tool_overrides }); setSaved(false);
                  }}>{t('nomi.robot.capabilities.restoreDefault')}</Button>
                </div>)}
            </section>
            <section className={styles.section}>
              <div className={styles.toolbar}>
                <h4>{t('nomi.robot.capabilities.presets')}</h4>
                <Button size='small' disabled={snapshot.tools.length === 0} onClick={addPreset}>{t('nomi.robot.capabilities.addPreset')}</Button>
              </div>
              <p className={styles.hint}>{t('nomi.robot.capabilities.presetsHint')}</p>
              {presetError && <p role='alert' className={styles.error}>{t('nomi.robot.capabilities.presetInvalid')}</p>}
              {draft.presets.map((preset, index) => <PresetEditor key={presetKeys[index]} preset={preset} rowKey={presetKeys[index]!}
                tools={snapshot.tools} onValidityChange={validity} onChange={(next) => {
                  setDraft({ ...draft, presets: draft.presets.map((item, itemIndex) => itemIndex === index ? next : item) }); setSaved(false);
                }} onRemove={() => {
                  setDraft({ ...draft, presets: draft.presets.filter((_, itemIndex) => itemIndex !== index) }); setSaved(false);
                  setPresetKeys((keys) => keys.filter((_, itemIndex) => itemIndex !== index));
                }} />)}
            </section>
          </div>
        </fieldset>
      </>}
      {!snapshot && busy == null && <Button onClick={() => void load()}>{t('nomi.robot.capabilities.reload')}</Button>}
    </div>
  </NomiModal>;
}
