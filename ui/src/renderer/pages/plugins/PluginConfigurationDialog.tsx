import { useEffect, useMemo, useState } from 'react';
import { Alert, Checkbox, Input, Modal, Select } from '@arco-design/web-react';
import { useTranslation } from 'react-i18next';
import { pluginPlatform } from '@/common/adapter/pluginPlatformBridge';
import type {
  ConfigurePluginRequest,
  PluginCredentialReference,
  PluginDetail,
} from '@/common/types/pluginPlatform';
import { isDesktopShell } from '@/renderer/utils/platform';
import styles from './PluginPlatform.module.css';

interface PluginConfigurationDialogProps {
  detail: PluginDetail | null;
  visible: boolean;
  loading: boolean;
  onCancel: () => void;
  onSubmit: (request: ConfigurePluginRequest) => void | Promise<void>;
}

export default function PluginConfigurationDialog({
  detail,
  visible,
  loading,
  onCancel,
  onSubmit,
}: PluginConfigurationDialogProps) {
  const { t } = useTranslation();
  const desktopShell = isDesktopShell();
  const [config, setConfig] = useState('{}');
  const [credentials, setCredentials] = useState<Record<string, string>>({});
  const [grants, setGrants] = useState<Record<string, boolean>>({});
  const [credentialOptions, setCredentialOptions] = useState<PluginCredentialReference[]>([]);
  const [error, setError] = useState('');

  useEffect(() => {
    if (!visible || !detail) return;
    setConfig(JSON.stringify(detail.config.values, null, 2));
    setCredentials(Object.fromEntries(detail.manifest.secret_slots.map((slot) => [
      slot,
      detail.credential_bindings.find((binding) => binding.slot === slot)?.credential_id ?? '',
    ])));
    setGrants(Object.fromEntries(detail.manifest.permissions.map((permission) => [
      permission,
      detail.grants.find((grant) => grant.permission === permission)?.granted ?? false,
    ])));
    setError('');
  }, [detail, visible]);

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

  const schema = useMemo(
    () => JSON.stringify(detail?.manifest.config_schema ?? { type: 'object' }, null, 2),
    [detail?.manifest.config_schema],
  );

  const submit = async () => {
    if (!desktopShell || !detail) return;
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
    const credential_bindings = Object.fromEntries(
      detail.manifest.secret_slots.map((slot) => [slot, credentials[slot]?.trim() || null]),
    );
    const unavailableSelection = Object.entries(credential_bindings).find(([, credentialId]) => (
      credentialId !== null
      && !credentialOptions.some((reference) => (
        reference.credential_id === credentialId && reference.enabled
      ))
    ));
    if (unavailableSelection) {
      setError(t('pluginPlatform.config.credentialUnavailableSelected', {
        slot: unavailableSelection[0],
      }));
      return;
    }
    const missingRequired = detail.credential_bindings.find(
      (binding) => binding.required && (
        !credential_bindings[binding.slot]
        || !credentialOptions.some((reference) => (
          reference.credential_id === credential_bindings[binding.slot] && reference.enabled
        ))
      ),
    );
    if (missingRequired) {
      setError(t('pluginPlatform.config.credentialRequired', { slot: missingRequired.slot }));
      return;
    }
    setError('');
    await onSubmit({
      expected_revision: detail.summary.revision,
      config: parsed as Record<string, unknown>,
      credential_bindings,
      grants,
    });
  };

  return (
    <Modal
      visible={visible}
      title={t('pluginPlatform.config.title')}
      okText={t('pluginPlatform.actions.save')}
      cancelText={t('pluginPlatform.actions.cancel')}
      confirmLoading={loading}
      okButtonProps={{ disabled: !desktopShell }}
      onCancel={loading ? undefined : onCancel}
      onOk={() => void submit()}
      unmountOnExit
    >
      <div className={styles.modalBody}>
        {!desktopShell && <Alert type='info' content={t('pluginPlatform.readOnly.body')} />}
        <Alert type='info' content={t('pluginPlatform.config.secretBoundary')} />
        {error && <Alert type='error' content={error} />}
        {!detail?.config.valid && detail?.config.validation_errors.length ? (
          <Alert type='warning' content={detail.config.validation_errors.join('\n')} />
        ) : null}
        <label>
          <strong>{t('pluginPlatform.config.values')}</strong>
          <Input.TextArea
            className={styles.jsonEditor}
            value={config}
            onChange={setConfig}
            disabled={!desktopShell}
            spellCheck={false}
            aria-label={t('pluginPlatform.config.values')}
          />
        </label>
        <details>
          <summary>{t('pluginPlatform.config.schema')}</summary>
          <pre>{schema}</pre>
        </details>
        {detail?.manifest.secret_slots.map((slot) => {
          const binding = detail.credential_bindings.find((value) => value.slot === slot);
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
          return (
            <label key={slot}>
              <strong>{slot}{binding?.required ? ' *' : ''}</strong>
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
            </label>
          );
        })}
        {detail?.manifest.permissions.map((permission) => (
          <Checkbox
            key={permission}
            disabled={!desktopShell}
            checked={grants[permission] ?? false}
            onChange={(checked) => setGrants((current) => ({ ...current, [permission]: checked }))}
          >
            {t('pluginPlatform.config.grantPermission', { permission })}
          </Checkbox>
        ))}
      </div>
    </Modal>
  );
}
