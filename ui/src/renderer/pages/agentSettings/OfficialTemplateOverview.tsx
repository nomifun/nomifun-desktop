import type {
  AgentCatalogResponse,
  ChatRouteRecord,
  OfficialPresetTemplate,
} from '@/common/types/agentPlatform';
import { AGENT_CHAT_MODEL_TASK } from '@/common/types/agentPlatform';
import { Alert, Button } from '@arco-design/web-react';
import { Copy, Lock } from '@icon-park/react';
import React, { useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import AgentCapabilityList from './AgentCapabilityList';
import { TEMPLATE_I18N_PATH } from './model';
import styles from './AgentSettingsPage.module.css';

type OfficialTemplateOverviewProps = {
  template: OfficialPresetTemplate;
  busy: boolean;
  catalog: AgentCatalogResponse;
  onFork: (
    displayName: string,
    modelRouteRefs: Record<string, string>,
    chatRouteRecords: Partial<Record<typeof AGENT_CHAT_MODEL_TASK, ChatRouteRecord>>
  ) => void;
};

const OfficialTemplateOverview: React.FC<OfficialTemplateOverviewProps> = ({
  template,
  busy,
  catalog,
  onFork,
}) => {
  const { t } = useTranslation();
  const path = TEMPLATE_I18N_PATH[template.template_key];
  const name = t(`agentSettings.template.${path}.name`);
  const unavailableTemplateCapabilities = useMemo(
    () =>
      [...template.seed.initial_capabilities, ...template.seed.on_demand_capabilities].filter(
        (reference) => {
          const item = catalog.capabilities.find(
            (candidate) =>
              candidate.capability.id === reference.id &&
              candidate.capability.version === reference.version
          );
          return item == null || item.materialization_state !== 'materialized';
        }
      ),
    [catalog.capabilities, template.seed.initial_capabilities, template.seed.on_demand_capabilities]
  );
  const templateUnavailable = unavailableTemplateCapabilities.length > 0;

  return (
    <main className={styles.editorSurface}>
      <header className={styles.editorHeader}>
        <div className={styles.editorHeaderCopy}>
          <div className={styles.eyebrow}>
            <Lock theme='outline' size='14' />
            {t('agentSettings.template.readOnly')}
          </div>
          <h2>{name}</h2>
          <p>{t(`agentSettings.template.${path}.description`)}</p>
        </div>
        <Button
          type='primary'
          icon={<Copy theme='outline' size='15' />}
          loading={busy}
          disabled={templateUnavailable}
          onClick={() =>
            onFork(t('agentSettings.defaults.forkName', { name }), {}, {})
          }
        >
          {t('agentSettings.actions.fork')}
        </Button>
      </header>

      {templateUnavailable && (
        <Alert
          className={styles.inlineNotice}
          type='warning'
          showIcon
          content={t('agentSettings.template.hostUnavailable')}
        />
      )}

      <section className={styles.section}>
        <div className={styles.sectionHeading}>
          <div>
            <h3>{t('agentSettings.sections.capabilities')}</h3>
            <p>{t('agentSettings.template.capabilityHint')}</p>
          </div>
        </div>
        <div className={styles.capabilityGroups}>
          <AgentCapabilityList
            title={t('agentSettings.capabilities.initial')}
            references={template.seed.initial_capabilities}
            catalog={catalog.capabilities}
          />
          <AgentCapabilityList
            title={t('agentSettings.capabilities.onDemand')}
            references={template.seed.on_demand_capabilities}
            catalog={catalog.capabilities}
          />
        </div>
        <div className={styles.requirementPolicy}>
          <strong>{t('agentSettings.resources.bindingPolicyTitle')}</strong>
          <span>{t('agentSettings.resources.bindingPolicyBody')}</span>
        </div>
      </section>

      {template.template_key === 'chat.minimal' && (
        <section className={styles.zeroToolBand}>
          <strong>{t('agentSettings.template.zeroToolTitle')}</strong>
          <span>{t('agentSettings.template.zeroToolBody')}</span>
        </section>
      )}
    </main>
  );
};

export default OfficialTemplateOverview;
