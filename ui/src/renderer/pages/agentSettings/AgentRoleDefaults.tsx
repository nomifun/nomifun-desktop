import { useRef, useState } from 'react';
import { Alert, Button, Modal, Spin } from '@arco-design/web-react';
import { useTranslation } from 'react-i18next';
import { agentPlatform } from '@/common/adapter/ipcBridge';
import type { AgentCatalogResponse, InstallationRoleBinding, RoleProviderSelection } from '@/common/types/agentPlatform';
import { providerSelectionKey } from './roleProviders';
import styles from './AgentSettingsPage.module.css';

export default function AgentRoleDefaults({ catalog }: { catalog: AgentCatalogResponse }) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [bindings, setBindings] = useState<InstallationRoleBinding[]>([]);
  const [choices, setChoices] = useState<Record<string, RoleProviderSelection>>({});
  const [loading, setLoading] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const loadGeneration = useRef(0);
  const [saving, setSaving] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const load = async () => {
    const generation = ++loadGeneration.current;
    setLoading(true); setLoaded(false); setError(null);
    try {
      const next = await agentPlatform.roleDefaults.invoke();
      if (generation !== loadGeneration.current) return;
      setBindings(next);
      setChoices(Object.fromEntries(next.map(binding => [binding.selection.role.key.role_id, binding.selection])));
      setLoaded(true);
    } catch { if (generation === loadGeneration.current) setError(t('agentSettings.providers.defaultsLoadError')); }
    finally { if (generation === loadGeneration.current) setLoading(false); }
  };
  const save = async (roleId: string) => {
    const selection = choices[roleId];
    if (!selection) return;
    setSaving(roleId); setError(null);
    try {
      const saved = await agentPlatform.putRoleDefault.invoke({ selection,
        expected_binding_version: bindings.find(binding => binding.selection.role.key.role_id === roleId)?.binding_version ?? 0 });
      setBindings(current => [...current.filter(binding => binding.selection.role.key.role_id !== roleId), saved]);
    } catch { setError(t('agentSettings.providers.defaultsSaveError')); }
    finally { setSaving(null); }
  };
  const roleIds = [...new Set([...catalog.roles.map(role => role.role.key.role_id), ...bindings.map(binding => binding.selection.role.key.role_id)])].sort();
  return <>
    <Button onClick={() => { setOpen(true); void load(); }}>{t('agentSettings.providers.defaultsTitle')}</Button>
    <Modal visible={open} title={t('agentSettings.providers.defaultsTitle')} footer={null}
      onCancel={() => { if (!saving) setOpen(false); }} autoFocus focusLock>
      <div style={{ maxHeight: '65vh', overflowY: 'auto' }}>
      <p>{t('agentSettings.providers.defaultsHint')}</p>
      {error && <Alert type='error' content={error} />}
      <Button disabled={loading || saving !== null} onClick={() => void load()}>{t('agentSettings.providers.defaultsReload')}</Button>
      {loading ? <Spin /> : loaded && <div className={styles.formGrid}>{roleIds.map(roleId => {
        const role = catalog.roles.find(item => item.role.key.role_id === roleId);
        const providers = role?.providers ?? [];
        const selection = choices[roleId];
        const key = selection ? providerSelectionKey(selection) : undefined;
        const current = bindings.find(binding => binding.selection.role.key.role_id === roleId);
        const missing = selection && !providers.some(item => providerSelectionKey(item.selection) === key);
        const label = role?.capabilities.map(ref => catalog.capabilities.find(item => item.capability.id === ref.id)?.display_name ?? ref.id).join(' / ') || roleId;
        return <div key={roleId} className={styles.field}>
          <span>{label}</span>
          <select className={styles.nativeSelect} aria-label={label} value={key ?? ''} disabled={saving !== null}
            onChange={(event) => {
              const value = event.currentTarget.value;
              const candidate = providers.find(item => providerSelectionKey(item.selection) === value);
              if (candidate) setChoices(previous => ({ ...previous, [roleId]: candidate.selection }));
            }}>
            {!selection && <option value=''>{t('agentSettings.providers.defaultsUnbound')}</option>}
            {missing && <option value={key!} disabled>{t('agentSettings.providers.missing')}</option>}
            {providers.map(item => <option key={providerSelectionKey(item.selection)} value={providerSelectionKey(item.selection)}>
              {item.display_name} — {item.source_package.id}@{item.source_package.version}
            </option>)}
          </select>
          {missing && <span role='alert'>{t('agentSettings.providers.defaultsMissingHint')}</span>}
          <Button disabled={saving !== null || !selection || !!missing || (current && providerSelectionKey(current.selection) === key)}
            loading={saving === roleId} onClick={() => void save(roleId)}>{t('agentSettings.providers.defaultsSave')}</Button>
        </div>;
      })}</div>}
      </div>
    </Modal>
  </>;
}
