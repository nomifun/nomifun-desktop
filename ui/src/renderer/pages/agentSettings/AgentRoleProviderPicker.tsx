import type { AgentCatalogResponse, AgentPresetDocument } from '@/common/types/agentPlatform';
import { Select } from '@arco-design/web-react';
import { useTranslation } from 'react-i18next';
import { providerSelectionKey, relevantRoleIds, selectRoleProvider } from './roleProviders';
import styles from './AgentSettingsPage.module.css';

type Props = {
  document: AgentPresetDocument;
  catalog: AgentCatalogResponse;
  disabled: boolean;
  onChange: (document: AgentPresetDocument) => void;
};

export default function AgentRoleProviderPicker({ document, catalog, disabled, onChange }: Props) {
  const { t } = useTranslation();
  const roleIds = relevantRoleIds(document, catalog);
  return <section className={styles.section} aria-label={t('agentSettings.providers.title')}>
    <div className={styles.sectionHeading}><div>
      <h3>{t('agentSettings.providers.title')}</h3>
      <p>{t('agentSettings.providers.hint')}</p>
    </div></div>
    {roleIds.length === 0 && <p className={styles.inlineEmpty}>{t('agentSettings.providers.empty')}</p>}
    <div className={styles.formGrid}>
      {roleIds.map(roleId => {
        const role = catalog.roles.find(item => item.role.key.role_id === roleId);
        const providers = role?.providers ?? [];
        const selection = document.system_role_provider_overrides[roleId];
        const value = selection ? providerSelectionKey(selection) : '';
        const selected = providers.find(item => providerSelectionKey(item.selection) === value);
        const missing = selection && !selected;
        const names = role?.capabilities.map(ref => catalog.capabilities.find(item =>
          item.capability.id === ref.id && item.capability.version === ref.version)?.display_name ?? ref.id);
        const label = names?.join(' / ') || roleId;
        return <div key={roleId} className={styles.field}>
          <span>{label}</span>
          <Select aria-label={label} value={value} disabled={disabled} onChange={(key: string) => {
            if (key === '') onChange(selectRoleProvider(document, roleId));
            else {
              const candidate = providers.find(item => providerSelectionKey(item.selection) === key);
              if (candidate) onChange(selectRoleProvider(document, roleId, candidate.selection));
            }
          }}>
            <Select.Option value=''>{t('agentSettings.providers.inherit')}</Select.Option>
            {missing && <Select.Option value={value} disabled>{t('agentSettings.providers.missing')}</Select.Option>}
            {providers.map(item => <Select.Option key={providerSelectionKey(item.selection)} value={providerSelectionKey(item.selection)}>
              {item.display_name} — {item.source_package.id}@{item.source_package.version}
            </Select.Option>)}
          </Select>
          {selected && <span className={styles.fieldHint}>{selected.description}</span>}
          {missing && <span role='alert' className={styles.fieldHint}>{t('agentSettings.providers.missingHint')}</span>}
        </div>;
      })}
    </div>
  </section>;
}
