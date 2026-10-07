import { useEffect, useRef, useState } from 'react';
import { Alert, Button, Checkbox, Input, Modal, Select, Spin } from '@arco-design/web-react';
import { ApplicationMenu, FileZip, FolderClose, Upload } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import { ipcBridge } from '@/common';
import { pluginPlatform } from '@/common/adapter/pluginPlatformBridge';
import type { PluginDetail, PluginCredentialReference, PluginImportInspection, PluginImportKind } from '@/common/types/pluginPlatform';
import { isDesktopShell } from '@/renderer/utils/platform';
import styles from './PluginDialogs.module.css';

interface PluginImportDialogProps {
  visible: boolean;
  onCancel: () => void;
  onInstalled: (plugin: PluginDetail) => void;
}

export default function PluginImportDialog({ visible, onCancel, onInstalled }: PluginImportDialogProps) {
  const { t } = useTranslation();
  const desktopShell = isDesktopShell();
  const [sourcePath, setSourcePath] = useState('');
  const [kind, setKind] = useState<PluginImportKind>('zip');
  const [inspection, setInspection] = useState<PluginImportInspection | null>(null);
  const [createCopy, setCreateCopy] = useState(false);
  const [config, setConfig] = useState('{}');
  const [credentials, setCredentials] = useState<Record<string, string>>({});
  const [credentialOptions, setCredentialOptions] = useState<PluginCredentialReference[]>([]);
  const [busy, setBusy] = useState<'reading' | 'importing' | null>(null);
  const [error, setError] = useState('');
  const generation = useRef(0);
  const existingSettings = useRef<{ config: string; credentials: Record<string, string> } | null>(null);

  useEffect(() => {
    generation.current += 1;
    if (!visible) return;
    setSourcePath(''); setInspection(null); setCreateCopy(false);
    setConfig('{}'); setCredentials({}); setCredentialOptions([]); setBusy(null); setError('');
    existingSettings.current = null;
    return () => { generation.current += 1; };
  }, [visible]);

  useEffect(() => {
    if (!visible || !desktopShell) return;
    let active = true;
    void pluginPlatform.credentials.list.invoke().then(references => {
      if (active) setCredentialOptions(references);
    }).catch(() => { if (active) setCredentialOptions([]); });
    return () => { active = false; };
  }, [desktopShell, visible]);

  const choose = async (nextKind: PluginImportKind) => {
    if (!desktopShell || busy) return;
    const request = ++generation.current;
    setBusy('reading'); setError('');
    try {
      const paths = await ipcBridge.dialog.showOpen.invoke(nextKind === 'directory'
        ? { properties: ['openDirectory'] }
        : { properties: ['openFile'], filters: [{ name: t(`pluginPlatform.import.${nextKind === 'backup' ? 'backup' : 'package'}`), extensions: ['zip'] }] });
      const selected = paths?.[0]?.trim();
      if (!selected || request !== generation.current) return;
      setSourcePath(selected); setKind(nextKind); setInspection(null);
      const inspected = await pluginPlatform.plugins.inspectImport.invoke({ source_path: selected, kind: nextKind });
      if (request !== generation.current) return;
      let settings = { config: '{}', credentials: {} as Record<string, string> };
      if (nextKind !== 'backup' && inspected.target_plugin_id) {
        const detail = await pluginPlatform.plugins.get.invoke({ plugin_id: inspected.target_plugin_id });
        if (request !== generation.current) return;
        settings = {
          config: JSON.stringify(detail.config.values, null, 2),
          credentials: Object.fromEntries(inspected.manifest.secret_slots.map(slot => [slot,
            detail.credential_bindings.find(binding => binding.slot === slot)?.credential_id ?? ''])),
        };
      }
      existingSettings.current = settings;
      setConfig(settings.config); setCredentials(settings.credentials); setCreateCopy(false); setInspection(inspected);
    } catch (caught) {
      if (request !== generation.current) return;
      console.error('[pluginPlatform] import inspection failed', caught);
      setError(t('pluginPlatform.import.invalid'));
    } finally { if (request === generation.current) setBusy(null); }
  };

  const install = async () => {
    if (!desktopShell || !inspection || !sourcePath || busy) return;
    let parsedConfig: Record<string, unknown> | undefined;
    if (kind !== 'backup') {
      let parsed: unknown;
      try { parsed = JSON.parse(config); }
      catch { setError(t('pluginPlatform.config.invalidJson')); return; }
      if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
        setError(t('pluginPlatform.config.objectRequired')); return;
      }
      parsedConfig = parsed as Record<string, unknown>;
    }
    const credentialBindings = Object.fromEntries(inspection.manifest.secret_slots.flatMap(slot => {
      const credentialId = credentials[slot]?.trim();
      return credentialId ? [[slot, credentialId]] : [];
    })) as Record<string, string>;
    const unavailableSlot = Object.entries(credentialBindings).find(([, credentialId]) =>
      !credentialOptions.some(reference => reference.credential_id === credentialId && reference.enabled))?.[0];
    if (unavailableSlot) {
      setError(t('pluginPlatform.config.credentialUnavailableSelected', { slot: unavailableSlot })); return;
    }
    const request = ++generation.current;
    setBusy('importing'); setError('');
    try {
      const response = await pluginPlatform.plugins.installImport.invoke({
        source_path: sourcePath, kind,
        ...(!createCopy && inspection.target_plugin_revision !== undefined ? { expected_plugin_revision: inspection.target_plugin_revision } : {}),
        create_copy: createCopy,
        ...(parsedConfig ? { config: parsedConfig } : {}),
        credential_bindings: credentialBindings,
      });
      if (request === generation.current) onInstalled(response.result.plugin);
    } catch (caught) {
      if (request !== generation.current) return;
      console.error('[pluginPlatform] import failed', caught); setError(t('pluginPlatform.import.failed'));
    } finally { if (request === generation.current) setBusy(null); }
  };

  const sources = [
    { kind: 'directory' as const, icon: <FolderClose />, label: 'directory', hint: 'directoryHint' },
    { kind: 'zip' as const, icon: <FileZip />, label: 'zip', hint: 'zipHint' },
    { kind: 'backup' as const, icon: <Upload />, label: 'backup', hint: 'backupHint' },
  ];
  return <Modal visible={visible} title={t('pluginPlatform.import.title')} style={{ width: 620 }}
    footer={<div className={styles.footer}>
      <Button disabled={Boolean(busy)} onClick={onCancel}>{t('pluginPlatform.actions.cancel')}</Button>
      <Button type='primary' disabled={!desktopShell || !inspection || Boolean(busy)} loading={busy === 'importing'} onClick={() => void install()}>
        {t('pluginPlatform.import.install')}
      </Button>
    </div>}
    onCancel={busy ? undefined : onCancel} maskClosable={false} unmountOnExit>
    <div className={styles.body}>
      <p className={styles.intro}>{t('pluginPlatform.import.body')}</p>
      {!desktopShell && <Alert type='info' content={t('pluginPlatform.readOnly.body')} />}
      <div className={styles.sources}>
        {sources.map(source => <button key={source.kind} type='button' className={styles.source}
          data-selected={inspection && kind === source.kind ? true : undefined}
          disabled={Boolean(busy) || !desktopShell} onClick={() => void choose(source.kind)}>
          <span className={styles.sourceIcon}>{source.icon}</span>
          <strong>{t(`pluginPlatform.import.${source.label}`)}</strong><small>{t(`pluginPlatform.import.${source.hint}`)}</small>
        </button>)}
      </div>
      {busy === 'reading' && <div className={styles.reading}><Spin size={18} /><span>{t('pluginPlatform.import.checking')}</span></div>}
      {error && <Alert type='error' content={error} />}
      {inspection ? <>
        <section className={styles.summary}>
          <span className={styles.appIcon}><ApplicationMenu /></span>
          <div className={styles.summaryCopy}><strong>{inspection.manifest.name}</strong><p>{inspection.manifest.description || t('pluginPlatform.import.descriptionFallback')}</p>
            <span className={styles.path} title={sourcePath}>{sourcePath}</span>
          </div>
          <span className={styles.kind}>{t(`pluginPlatform.shape.${inspection.manifest.entrypoints.ui ? inspection.manifest.entrypoints.service ? 'mixed' : 'ui_only' : 'headless'}`)}</span>
        </section>
        {inspection.backup && <p className={styles.note}>{t('pluginPlatform.import.backupDisclosure', { files: inspection.backup.file_count })}</p>}
        {inspection.target_plugin_id && <label className={styles.copyOption}>
          <Checkbox checked={createCopy} disabled={Boolean(busy)} onChange={checked => {
            setCreateCopy(checked);
            setConfig(checked ? '{}' : existingSettings.current?.config ?? '{}');
            setCredentials(checked ? {} : existingSettings.current?.credentials ?? {});
          }}>{t('pluginPlatform.import.createCopy')}</Checkbox>
          <span>{t(createCopy ? 'pluginPlatform.import.copyHint' : kind === 'backup' ? 'pluginPlatform.import.backupReplaceHint' : 'pluginPlatform.import.replaceHint')}</span>
        </label>}
        {inspection.manifest.secret_slots.length > 0 && <section className={styles.fields}>
          <div className={styles.sectionHeading}><strong>{t('pluginPlatform.config.credentials')}</strong><p>{t('pluginPlatform.config.credentialsHint')}</p></div>
          {inspection.backup?.credential_slots_to_rebind.length ? <p className={styles.note}>{t('pluginPlatform.import.rebindCredentials', { slots: inspection.backup.credential_slots_to_rebind.join(', ') })}</p> : null}
          {inspection.manifest.secret_slots.map(slot => {
            const selected = credentials[slot]?.trim() ?? '';
            const options = credentialOptions.map(reference => ({ value: reference.credential_id,
              label: reference.enabled ? reference.label : `${reference.label} (${t('pluginPlatform.config.credentialUnavailable')})`, disabled: !reference.enabled }));
            if (selected && !options.some(option => option.value === selected)) options.push({ value: selected, label: `${selected} (${t('pluginPlatform.config.credentialUnavailable')})`, disabled: true });
            return <label key={slot} className={styles.field}><span>{slot}</span>
              <Select allowClear showSearch disabled={!desktopShell || Boolean(busy)} value={selected || undefined}
                onChange={value => setCredentials(current => ({ ...current, [slot]: typeof value === 'string' ? value : '' }))}
                placeholder={t('pluginPlatform.config.credentialReference')} aria-label={t('pluginPlatform.config.credentialSlot', { slot })} options={options} />
            </label>;
          })}
        </section>}
        <details className={styles.advanced}>
          <summary>{t('pluginPlatform.config.advanced')}</summary>
          {kind !== 'backup' && <label className={styles.field}><span>{t('pluginPlatform.config.values')}</span>
            <Input.TextArea className={styles.jsonEditor} value={config} onChange={setConfig} disabled={!desktopShell || Boolean(busy)} spellCheck={false}
              aria-label={t('pluginPlatform.config.values')} autoSize={{ minRows: 4, maxRows: 8 }} />
          </label>}
          <dl className={styles.facts}>
            <div><dt>{t('pluginPlatform.detail.actions')}</dt><dd>{inspection.manifest.actions.map(action => action.name).join(', ') || '—'}</dd></div>
            <div><dt>{t('pluginPlatform.detail.bindings')}</dt><dd>{inspection.manifest.bindings.map(binding => binding.point).join(', ') || '—'}</dd></div>
            {inspection.manifest.permissions.length > 0 && <div><dt>{t('pluginPlatform.import.permissions')}</dt><dd>{inspection.manifest.permissions.join(', ')}</dd></div>}
          </dl>
        </details>
      </> : !busy && !error ? <div className={styles.empty}><Upload /><strong>{t('pluginPlatform.import.emptyTitle')}</strong><span>{t('pluginPlatform.import.emptyBody')}</span></div> : null}
    </div>
  </Modal>;
}
