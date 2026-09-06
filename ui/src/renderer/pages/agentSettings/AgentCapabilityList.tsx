import type {
  CapabilityCatalogItem,
  CapabilityPlacement,
  ExactCatalogRef,
} from '@/common/types/agentPlatform';
import { Radio, Tag } from '@arco-design/web-react';
import React, { useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import {
  capabilityProductCopy,
  capabilityReferenceKey,
  humanizeResourceKind,
  RESOURCE_KIND_I18N_KEYS,
} from './model';
import styles from './AgentSettingsPage.module.css';

type CapabilityReference = ExactCatalogRef<'capability'>;

type AgentCapabilityListProps = {
  title?: string;
  references: readonly CapabilityReference[];
  catalog: readonly CapabilityCatalogItem[];
  emptyLabel?: string;
  disabled?: boolean;
  placementFor?: (capability: CapabilityReference) => CapabilityPlacement;
  onPlacementChange?: (
    capability: CapabilityReference,
    placement: CapabilityPlacement
  ) => void;
};

const SOURCE_KIND_LABELS: Record<string, { en: string; zh: string }> = {
  bundled: { en: 'Built-in', zh: '内置' },
  first_party: { en: 'Built-in', zh: '内置' },
  managed_local: { en: 'Local package', zh: '本地包' },
  mcp_binding: { en: 'MCP connection', zh: 'MCP 连接' },
  platform_builtin: { en: 'Built-in', zh: '内置' },
  plugin_mount: { en: 'Plugin', zh: '插件' },
  test_fixture: { en: 'Test fixture', zh: '测试夹具' },
};

const sourceKindLabel = (sourceKind: string, language: string): string => {
  const labels = SOURCE_KIND_LABELS[sourceKind.trim().toLowerCase()];
  if (labels) return language.toLowerCase().startsWith('zh') ? labels.zh : labels.en;
  return humanizeResourceKind(sourceKind);
};

const AgentCapabilityList: React.FC<AgentCapabilityListProps> = ({
  title,
  references,
  catalog,
  emptyLabel,
  disabled = false,
  placementFor,
  onPlacementChange,
}) => {
  const { t, i18n } = useTranslation();
  const catalogByReference = useMemo(
    () =>
      new Map(
        catalog.map((item) => [capabilityReferenceKey(item.capability), item] as const)
      ),
    [catalog]
  );
  const editable = Boolean(placementFor && onPlacementChange);
  const resourceKindLabel = (resourceKind: string): string => {
    const key = RESOURCE_KIND_I18N_KEYS[resourceKind];
    return key
      ? t(`agentSettings.resources.kinds.${key}`, {
          defaultValue: humanizeResourceKind(resourceKind),
        })
      : humanizeResourceKind(resourceKind);
  };
  return (
    <div className={styles.capabilityGroup}>
      {title && (
        <div className={styles.capabilityGroupHeader}>
          <strong>{title}</strong>
          <span>{references.length}</span>
        </div>
      )}
      {references.length === 0 ? (
        <div className={styles.inlineEmpty}>
          {emptyLabel ?? t('agentSettings.capabilities.emptySelection')}
        </div>
      ) : (
        <div className={styles.capabilityList}>
          {references.map((reference) => {
            const item = catalogByReference.get(capabilityReferenceKey(reference));
            const unavailable = item == null || item.materialization_state !== 'materialized';
            const placement = placementFor?.(reference) ?? 'none';
            const copy = item
              ? capabilityProductCopy(item, i18n.language)
              : {
                  name: reference.id,
                  description: t('agentSettings.capabilities.catalogUnavailable'),
                };
            const displayName = copy.name;
            const source = item
              ? `${sourceKindLabel(item.source_kind, i18n.language)} · ${item.source_package.id}@${item.source_package.version}`
              : `${t('agentSettings.common.unavailable')} · ${reference.id}@${reference.version}`;

            return (
              <div
                key={capabilityReferenceKey(reference)}
                className={styles.capabilityRow}
              >
                <div className={styles.capabilityMain}>
                  <div className={styles.capabilityTitle}>
                    <strong>{displayName}</strong>
                    <code className={styles.capabilityIdentity}>
                      {capabilityReferenceKey(reference)}
                    </code>
                    <Tag size='small' color={unavailable ? 'orange' : 'green'}>
                      {t(
                        unavailable
                          ? 'agentSettings.common.unavailable'
                          : 'agentSettings.common.available'
                      )}
                    </Tag>
                  </div>
                  <p>{copy.description}</p>
                  {unavailable && (
                    <div className={styles.capabilityUnavailableReason}>
                      {t('agentSettings.capabilities.unavailableReason')}
                    </div>
                  )}
                  <div className={styles.capabilityMetadata}>
                    <span>
                      {t('agentSettings.capabilities.source')}: {source}
                    </span>
                    {item && item.action_count > 0 && (
                      <span>
                        {t('agentSettings.capabilities.tools', {
                          count: item.action_count,
                        })}
                      </span>
                    )}
                    {item && item.context_contributor_count > 0 && (
                      <span>
                        {t('agentSettings.capabilities.contexts', {
                          count: item.context_contributor_count,
                        })}
                      </span>
                    )}
                  </div>
                  <div className={styles.capabilityResources}>
                    <span>{t('agentSettings.resources.requiredAtUse')}</span>
                    <div className={styles.tagRow}>
                      {item == null ? (
                        <span className={styles.capabilityNoResource}>
                          {t('agentSettings.common.unavailable')}
                        </span>
                      ) : item.required_resource_kinds.length > 0 ? (
                        item.required_resource_kinds.map((resourceKind) => (
                          <Tag key={resourceKind} size='small' color='gray'>
                            {resourceKindLabel(resourceKind)}
                          </Tag>
                        ))
                      ) : (
                        <span className={styles.capabilityNoResource}>
                          {t('agentSettings.resources.noneRequired')}
                        </span>
                      )}
                    </div>
                  </div>
                </div>

                {editable && (
                  <Radio.Group
                    type='button'
                    size='small'
                    className={styles.capabilityMode}
                    value={placement}
                    disabled={disabled}
                    aria-label={t('agentSettings.capabilities.modeAria', {
                      name: displayName,
                    })}
                    onChange={(value: CapabilityPlacement) =>
                      onPlacementChange?.(reference, value)
                    }
                  >
                    <Radio value='none'>
                      {t('agentSettings.capabilities.notSelected')}
                    </Radio>
                    <Radio value='initial' disabled={unavailable}>
                      {t('agentSettings.capabilities.initialShort')}
                    </Radio>
                    <Radio value='on_demand' disabled={unavailable}>
                      {t('agentSettings.capabilities.onDemandShort')}
                    </Radio>
                  </Radio.Group>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
};

export default AgentCapabilityList;
