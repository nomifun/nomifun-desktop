import type {
  OfficialPresetTemplate,
  TemplateResourceSelection,
} from '@/common/types/agentPlatform';
import { Button, Input, Tag } from '@arco-design/web-react';
import { Copy, Lock } from '@icon-park/react';
import React, { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { TEMPLATE_I18N_PATH } from './model';
import styles from './AgentSettingsPage.module.css';

type OfficialTemplateOverviewProps = {
  template: OfficialPresetTemplate;
  busy: boolean;
  onFork: (
    displayName: string,
    resources: TemplateResourceSelection[],
    modelRouteRefs: Record<string, string>
  ) => void;
};

const ExactRefList: React.FC<{
  title: string;
  items: Array<{ id: string; version: string }>;
  emptyLabel: string;
}> = ({ title, items, emptyLabel }) => (
  <div className={styles.templateColumn}>
    <div className={styles.templateColumnHeader}>
      <span>{title}</span>
      <span>{items.length}</span>
    </div>
    {items.length === 0 ? (
      <div className={styles.inlineEmpty}>{emptyLabel}</div>
    ) : (
      <div className={styles.exactList}>
        {items.map((item) => (
          <div key={`${item.id}@${item.version}`} className={styles.exactRow}>
            <span>{item.id}</span>
            <code>{item.version}</code>
          </div>
        ))}
      </div>
    )}
  </div>
);

const OfficialTemplateOverview: React.FC<OfficialTemplateOverviewProps> = ({
  template,
  busy,
  onFork,
}) => {
  const { t } = useTranslation();
  const path = TEMPLATE_I18N_PATH[template.template_key];
  const name = t(`agentSettings.template.${path}.name`);
  const [resourceIds, setResourceIds] = useState<Record<string, string>>({});
  const [chatModelRoute, setChatModelRoute] = useState('');

  useEffect(() => {
    setResourceIds({});
    setChatModelRoute('');
  }, [template.template_key]);

  const resources = useMemo(
    () =>
      template.seed.typed_resource_defaults
        .map((resource): TemplateResourceSelection | null => {
          const resourceId = resourceIds[resource.slot_key]?.trim();
          if (!resourceId) return null;
          return {
            slot_key: resource.slot_key,
            resource_kind: resource.resource_kind,
            resource_id: resourceId,
            typed_parameters: {},
          };
        })
        .filter((resource): resource is TemplateResourceSelection => resource !== null),
    [resourceIds, template.seed.typed_resource_defaults]
  );
  const missingRequired = template.seed.typed_resource_defaults.some(
    (resource) => resource.required && !resourceIds[resource.slot_key]?.trim()
  );

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
          <div className={styles.tagRow}>
            {template.role_coverage.required_capability_categories.map((category) => (
              <Tag key={category} size='small' color='gray'>
                {category}
              </Tag>
            ))}
          </div>
        </div>
        <Button
          type='primary'
          icon={<Copy theme='outline' size='15' />}
          loading={busy}
          disabled={missingRequired}
          onClick={() =>
            onFork(
              t('agentSettings.defaults.forkName', { name }),
              resources,
              chatModelRoute.trim() ? { chat: chatModelRoute.trim() } : {}
            )
          }
        >
          {t('agentSettings.actions.fork')}
        </Button>
      </header>

      <section className={styles.section}>
        <div className={styles.sectionHeading}>
          <div>
            <h3>{t('agentSettings.sections.capabilities')}</h3>
            <p>{t('agentSettings.template.capabilityHint')}</p>
          </div>
        </div>
        <div className={styles.dualGrid}>
          <ExactRefList
            title={t('agentSettings.capabilities.initial')}
            items={template.seed.initial_capabilities}
            emptyLabel={t('agentSettings.common.none')}
          />
          <ExactRefList
            title={t('agentSettings.capabilities.onDemand')}
            items={template.seed.on_demand_capabilities}
            emptyLabel={t('agentSettings.common.none')}
          />
        </div>
      </section>

      <section className={styles.section}>
        <div className={styles.sectionHeading}>
          <div>
            <h3>{t('agentSettings.sections.resources')}</h3>
            <p>{t('agentSettings.template.resourceHint')}</p>
          </div>
        </div>
        <label className={styles.field}>
          <span>{t('agentSettings.fields.chatModelRoute')}</span>
          <Input
            value={chatModelRoute}
            placeholder={t('agentSettings.fields.chatModelRoutePlaceholder')}
            onChange={setChatModelRoute}
          />
        </label>
        {template.seed.typed_resource_defaults.length === 0 ? (
          <div className={styles.inlineEmpty}>{t('agentSettings.resources.noneRequired')}</div>
        ) : (
          <div className={styles.resourceDefaults}>
            {template.seed.typed_resource_defaults.map((resource) => (
              <div key={resource.slot_key} className={styles.resourceDefaultRow}>
                <div>
                  <strong>{resource.slot_key}</strong>
                  <span>{resource.resource_kind}</span>
                </div>
                <Input
                  className={styles.templateResourceInput}
                  value={resourceIds[resource.slot_key] ?? ''}
                  placeholder={t('agentSettings.resources.resourceId')}
                  onChange={(resourceId) =>
                    setResourceIds((current) => ({
                      ...current,
                      [resource.slot_key]: resourceId,
                    }))
                  }
                />
                <div className={styles.tagRow}>
                  {resource.required && (
                    <Tag size='small' color='red'>
                      {t('agentSettings.resources.required')}
                    </Tag>
                  )}
                  {resource.operations.map((operation) => (
                    <Tag key={operation} size='small' color='gray'>
                      {operation}
                    </Tag>
                  ))}
                </div>
              </div>
            ))}
          </div>
        )}
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
