import { useEffect, useState } from 'react';
import { Alert, Button, Checkbox, Input, Modal, Select, Spin, Tag } from '@arco-design/web-react';
import { FileZip, FolderClose, Upload } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import { ipcBridge } from '@/common';
import { pluginPlatform } from '@/common/adapter/pluginPlatformBridge';
import type {
  PluginDetail,
  PluginCredentialReference,
  PluginImportInspection,
  PluginImportKind,
  PluginPermissionExpansion,
} from '@/common/types/pluginPlatform';
import { isDesktopShell } from '@/renderer/utils/platform';
import styles from './PluginPlatform.module.css';

interface PluginImportDialogProps {
  visible: boolean;
  onCancel: () => void;
  onInstalled: (plugin: PluginDetail) => void;
}

export default function PluginImportDialog({
  visible,
  onCancel,
  onInstalled,
}: PluginImportDialogProps) {
  const { t } = useTranslation();
  const desktopShell = isDesktopShell();
  const [sourcePath, setSourcePath] = useState('');
  const [kind, setKind] = useState<PluginImportKind>('zip');
  const [inspection, setInspection] = useState<PluginImportInspection | null>(null);
  const [confirmation, setConfirmation] = useState<PluginPermissionExpansion | null>(null);
  const [createCopy, setCreateCopy] = useState(false);
  const [config, setConfig] = useState('{}');
  const [credentials, setCredentials] = useState<Record<string, string>>({});
  const [credentialOptions, setCredentialOptions] = useState<PluginCredentialReference[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');

  useEffect(() => {
    if (!visible) return;
    setSourcePath('');
    setInspection(null);
    setConfirmation(null);
    setCreateCopy(false);
    setConfig('{}');
    setCredentials({});
    setBusy(false);
    setError('');
  }, [visible]);

  useEffect(() => {
    if (!visible || !desktopShell) return;
    let active = true;
    void pluginPlatform.credentials.list.invoke().then((references) => {
      if (active) setCredentialOptions(references);
    }).catch(() => {
      if (active) setCredentialOptions([]);
    });
    return () => { active = false; };
  }, [desktopShell, visible]);

  const choose = async (nextKind: PluginImportKind) => {
    if (!desktopShell) return;
    setBusy(true);
    setError('');
    setInspection(null);
    setConfirmation(null);
    try {
      const paths = await ipcBridge.dialog.showOpen.invoke(
        nextKind === 'directory'
          ? { properties: ['openDirectory'] }
          : {
              properties: ['openFile'],
              filters: [{
                name: nextKind === 'backup'
                  ? t('pluginPlatform.import.backup')
                  : t('pluginPlatform.import.package'),
                extensions: ['zip'],
              }],
            },
      );
      const selected = paths?.[0]?.trim();
      if (!selected) return;
      setSourcePath(selected);
      setKind(nextKind);
      const inspected = await pluginPlatform.plugins.inspectImport.invoke({
        source_path: selected,
        kind: nextKind,
      });
      setInspection(inspected);
      setConfirmation(inspected.permission_expansion ?? null);
      setConfig('{}');
      setCredentials({});
      if (nextKind !== 'backup' && inspected.target_plugin_id) {
        const detail = await pluginPlatform.plugins.get.invoke({
          plugin_id: inspected.target_plugin_id,
        });
        setConfig(JSON.stringify(detail.config.values, null, 2));
        setCredentials(Object.fromEntries(detail.manifest.secret_slots.map((slot) => [
          slot,
          detail.credential_bindings.find((binding) => binding.slot === slot)?.credential_id ?? '',
        ])));
      }
    } catch (caught) {
      console.error('[pluginPlatform] import inspection failed', caught);
      setError(t('pluginPlatform.import.invalid'));
    } finally {
      setBusy(false);
    }
  };

  const install = async (permissionConfirmationId?: string) => {
    if (!desktopShell || !inspection || !sourcePath || busy) return;
    let parsedConfig: Record<string, unknown> | undefined;
    if (kind !== 'backup') {
      let parsed: unknown;
      try {
        parsed = JSON.parse(config);
      } catch {
        setError(t('pluginPlatform.config.invalidJson'));
        return;
      }
      if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
        setError(t('pluginPlatform.config.objectRequired'));
        return;
      }
      parsedConfig = parsed as Record<string, unknown>;
    }
    const credentialBindings = Object.fromEntries(
      inspection.manifest.secret_slots.flatMap((slot) => {
        const credentialId = credentials[slot]?.trim();
        return credentialId ? [[slot, credentialId]] : [];
      }),
    ) as Record<string, string>;
    const unavailableSlot = Object.entries(credentialBindings).find(([, credentialId]) => (
      !credentialOptions.some((reference) => (
        reference.credential_id === credentialId && reference.enabled
      ))
    ))?.[0];
    if (unavailableSlot) {
      setError(t('pluginPlatform.config.credentialUnavailableSelected', {
        slot: unavailableSlot,
      }));
      return;
    }
    setBusy(true);
    setError('');
    try {
      const response = await pluginPlatform.plugins.installImport.invoke({
        source_path: sourcePath,
        kind,
        ...(inspection.target_plugin_revision === undefined
          ? {}
          : { expected_plugin_revision: inspection.target_plugin_revision }),
        create_copy: createCopy,
        ...(permissionConfirmationId
          ? { permission_confirmation_id: permissionConfirmationId }
          : {}),
        ...(parsedConfig ? { config: parsedConfig } : {}),
        credential_bindings: credentialBindings,
      });
      if (response.result.outcome === 'confirmation_required') {
        setConfirmation(response.result.confirmation);
        return;
      }
      onInstalled(response.result.plugin);
    } catch (caught) {
      console.error('[pluginPlatform] import failed', caught);
      setError(t('pluginPlatform.import.failed'));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal
      visible={visible}
      title={t('pluginPlatform.import.title')}
      footer={null}
      onCancel={busy ? undefined : onCancel}
      maskClosable={false}
      unmountOnExit
    >
      <div className={styles.modalBody}>
        {!desktopShell && <Alert type='info' content={t('pluginPlatform.readOnly.body')} />}
        <p>{t('pluginPlatform.import.body')}</p>
        <div className={styles.actions}>
          <Button icon={<FolderClose />} disabled={busy || !desktopShell} onClick={() => void choose('directory')}>
            {t('pluginPlatform.import.directory')}
          </Button>
          <Button icon={<FileZip />} disabled={busy || !desktopShell} onClick={() => void choose('zip')}>
            {t('pluginPlatform.import.zip')}
          </Button>
          <Button icon={<Upload />} disabled={busy || !desktopShell} onClick={() => void choose('backup')}>
            {t('pluginPlatform.import.backup')}
          </Button>
        </div>
        {busy && <div><Spin /> {t('pluginPlatform.import.checking')}</div>}
        {error && <Alert type='error' content={error} />}
        {inspection && (
          <section className={styles.disclosure}>
            <div className={styles.sectionHeader}>
              <div>
                <strong>{inspection.manifest.name}</strong>
                <div className={styles.muted}>
                  {inspection.manifest.package_id} · {inspection.manifest.version}
                </div>
              </div>
              <Tag>{inspection.kind}</Tag>
            </div>
            <p>{inspection.manifest.description}</p>
            <div>{t('pluginPlatform.import.shape', {
              ui: inspection.manifest.entrypoints.ui ? t('pluginPlatform.shape.ui') : '—',
              service: inspection.manifest.entrypoints.service ? t('pluginPlatform.shape.service') : '—',
            })}</div>
            {inspection.trusted_local_service && (
              <Alert type='warning' content={t('pluginPlatform.import.trustedService')} />
            )}
            {inspection.manifest.permissions.length > 0 && (
              <div>
                <strong>{t('pluginPlatform.import.permissions')}</strong>
                <div>{inspection.manifest.permissions.join(', ')}</div>
              </div>
            )}
            <div>
              <strong>{t('pluginPlatform.detail.actions')}</strong>
              <div>{inspection.manifest.actions.map((action) => action.name).join(', ') || '—'}</div>
            </div>
            <div>
              <strong>{t('pluginPlatform.detail.bindings')}</strong>
              <div>{inspection.manifest.bindings.map((binding) => binding.point).join(', ') || '—'}</div>
            </div>
            {inspection.backup && (
              <Alert
                type='info'
                content={t('pluginPlatform.import.backupDisclosure', {
                  files: inspection.backup.file_count,
                  bytes: inspection.backup.database_size_bytes,
                })}
              />
            )}
            {inspection.backup?.credential_slots_to_rebind.length ? (
              <Alert
                type='warning'
                content={t('pluginPlatform.import.rebindCredentials', {
                  slots: inspection.backup.credential_slots_to_rebind.join(', '),
                })}
              />
            ) : null}
            {kind !== 'backup' && <label>
              <strong>{t('pluginPlatform.config.values')}</strong>
              <Input.TextArea
                className={styles.jsonEditor}
                value={config}
                onChange={setConfig}
                disabled={!desktopShell}
                spellCheck={false}
                aria-label={t('pluginPlatform.config.values')}
              />
            </label>}
            {inspection.manifest.secret_slots.map((slot) => {
              const selected = credentials[slot]?.trim() ?? '';
              const options = credentialOptions.map((reference) => ({
                value: reference.credential_id,
                label: reference.enabled
                  ? `${reference.label} · ${reference.kind}`
                  : `${reference.label} · ${reference.kind} (${t('pluginPlatform.config.credentialUnavailable')})`,
                disabled: !reference.enabled,
              }));
              if (selected && !options.some((option) => option.value === selected)) {
                options.push({
                  value: selected,
                  label: `${selected} (${t('pluginPlatform.config.credentialUnavailable')})`,
                  disabled: true,
                });
              }
              return <label key={slot}>
                <strong>{slot}</strong>
                <Select
                  allowClear
                  showSearch
                  disabled={!desktopShell}
                  value={credentials[slot] ?? ''}
                  onChange={(value) => setCredentials((current) => ({
                    ...current,
                    [slot]: typeof value === 'string' ? value : '',
                  }))}
                  placeholder={t('pluginPlatform.config.credentialReference')}
                  aria-label={t('pluginPlatform.config.credentialSlot', { slot })}
                  options={options}
                />
              </label>;
            })}
            <code>{inspection.artifact_digest.slice(0, 16)}…</code>
            {inspection.target_plugin_id && (
              <Checkbox checked={createCopy} onChange={(checked) => {
                setCreateCopy(checked);
                if (checked) {
                  setConfig('{}');
                  setCredentials({});
                }
              }}>
                {t('pluginPlatform.import.createCopy')}
              </Checkbox>
            )}
            <Button type='primary' long loading={busy} disabled={!desktopShell} onClick={() => void install()}>
              {t('pluginPlatform.import.install')}
            </Button>
          </section>
        )}
        {confirmation && (
          <section className={`${styles.disclosure} ${styles.warning}`}>
            <strong>{t('pluginPlatform.permissions.title')}</strong>
            {confirmation.added_permissions.length > 0 && (
              <div>{t('pluginPlatform.permissions.added', {
                permissions: confirmation.added_permissions.join(', '),
              })}</div>
            )}
            {confirmation.added_secret_slots.length > 0 && (
              <div>{t('pluginPlatform.permissions.secrets', {
                slots: confirmation.added_secret_slots.join(', '),
              })}</div>
            )}
            {confirmation.trusted_local_service && (
              <Alert type='warning' content={t('pluginPlatform.permissions.localCode')} />
            )}
            <Button
              status='warning'
              loading={busy}
              disabled={!desktopShell}
              onClick={() => void install(confirmation.confirmation_id)}
            >
              {t('pluginPlatform.permissions.confirm')}
            </Button>
          </section>
        )}
      </div>
    </Modal>
  );
}
