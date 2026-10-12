import type { AgentCatalogResponse, AgentPresetRecommendedCapabilitiesResponse } from '@/common/types/agentPlatform';
import { Button } from '@arco-design/web-react';
import React from 'react';
import { useTranslation } from 'react-i18next';
import { actionFallbackName, actionI18nKey, moduleI18nKey } from './model';
import styles from './AgentSettingsPage.module.css';

type Props = {
  recommendation: AgentPresetRecommendedCapabilitiesResponse;
  catalog: AgentCatalogResponse;
  busy: boolean;
  saving: boolean;
  onApply: () => void;
  onDismiss: () => void;
};

/** Review the explicit additions without replacing the user's other settings. */
const AgentRecommendedCapabilities: React.FC<Props> = ({ recommendation, catalog, busy, saving, onApply, onDismiss }) => {
  const { t } = useTranslation();
  const changed = recommendation.added_capabilities.length > 0 || recommendation.added_skills.length > 0;
  return <section className={styles.recommendation} aria-label={t('agentSettings.recommendation.title')}>
    <strong>{t(changed ? 'agentSettings.recommendation.title' : 'agentSettings.recommendation.current')}</strong>
    {changed && <>
      <p>{t('agentSettings.recommendation.reviewHint')}</p>
      <ul className={styles.recommendationList}>
        {recommendation.added_capabilities.map(selection => {
          const id = String(selection.capability.id);
          const module = catalog.modules.find(item => item.module.id === id);
          const key = moduleI18nKey(id);
          const name = key ? t(`agentSettings.modules.${key}.name`, { defaultValue: module?.display_name ?? id }) : module?.display_name ?? id;
          const actionNames = (selection.action_allowlist ?? []).map(action => t(
            action.startsWith('plugin.development/')
              ? `pluginPlatform.authoring.operations.${action.slice('plugin.development/'.length)}`
              : `agentSettings.actionLabels.${actionI18nKey(action)}`,
            { defaultValue: actionFallbackName(action) }
          ));
          return <li key={id}><strong>{name}</strong>{actionNames.length > 0 && <span>{actionNames.join(' · ')}</span>}</li>;
        })}
        {recommendation.added_skills.map(skill => <li key={skill.id}>
          <strong>{catalog.skills.find(item => item.skill.id === skill.id)?.display_name ?? skill.id}</strong>
          <span>{t('agentSettings.recommendation.skill', { version: skill.version })}</span>
        </li>)}
      </ul>
      <p>{t('agentSettings.recommendation.sessionHint')}</p>
    </>}
    <div className={styles.recommendationActions}>
      <Button size='small' disabled={busy} onClick={onDismiss}>{t('common.close')}</Button>
      {changed && <Button size='small' type='primary' loading={saving} disabled={busy} onClick={onApply}>{t('agentSettings.recommendation.apply')}</Button>}
    </div>
  </section>;
};

export default AgentRecommendedCapabilities;
