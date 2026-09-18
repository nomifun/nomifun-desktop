import type {
  AgentCatalogResponse,
  AgentPresetDocument,
  CapabilityModuleAction,
  CapabilityModuleCatalogItem,
} from '@/common/types/agentPlatform';
import { Button, Checkbox, Modal, Tag } from '@arco-design/web-react';
import {
  Book,
  Check,
  Code,
  Connection,
  Down,
  Earth,
  Info,
  Lightning,
  Magic,
  Refresh,
  Search,
  User,
} from '@icon-park/react';
import React, { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useSearchParams } from 'react-router-dom';
import {
  MODULE_CATEGORIES,
  isBuiltinModule,
  moduleAvailability,
  moduleCategory,
  moduleIsAvailable,
  moduleIsSelectable,
  selectedModuleReferences,
  type ModuleCategory,
  type ModuleReference,
} from './capabilityGroups';
import {
  actionFallbackName,
  capabilityReferenceKey,
  humanizeResourceKind,
  moduleI18nKey,
  moduleMatchesSearch,
  RESOURCE_KIND_I18N_KEYS,
} from './model';
import { planModuleChange, setModuleActions } from './capabilityChanges';
import styles from './AgentCapabilityWorkspace.module.css';

const categoryIcons = {
  knowledge: Book,
  development: Code,
  web: Earth,
  collaboration: User,
  creation: Magic,
  automation: Lightning,
  devices: Connection,
  integrations: Connection,
};

export const ModuleCategoryIcon: React.FC<{ category: ModuleCategory; size?: number }> = ({
  category,
  size = 16,
}) => {
  const Icon = categoryIcons[category];
  return <Icon theme='outline' size={size} />;
};

type MissingModule = {
  module: ModuleReference;
  display_name: string;
  description: string;
  missing: true;
  savedActions: string[];
};

type ModuleEntry =
  | { module: CapabilityModuleCatalogItem; missing: false; savedActions: string[] }
  | MissingModule;

type Props = {
  document: AgentPresetDocument;
  catalog: AgentCatalogResponse;
  disabled?: boolean;
  onChange: (document: AgentPresetDocument) => void;
};

const actionTranslationKey = (actionId: string): string =>
  actionId.slice(actionId.indexOf('/') + 1).replace(/[^a-zA-Z0-9]+/g, '_');

const effectTone = (effectClass: string): 'critical' | 'caution' | 'normal' => {
  if (['destructive', 'irreversible', 'physical'].includes(effectClass)) return 'critical';
  if (['external_transmit', 'execute_local', 'write_durable'].includes(effectClass)) return 'caution';
  return 'normal';
};

const AgentCapabilityWorkspace: React.FC<Props> = ({
  document,
  catalog,
  disabled = false,
  onChange,
}) => {
  const { t } = useTranslation();
  const [searchParams, setSearchParams] = useSearchParams();
  const [query, setQuery] = useState('');
  const [category, setCategory] = useState<ModuleCategory | 'all'>('all');
  const [status, setStatus] = useState<'all' | 'enabled' | 'attention'>('all');
  const [pluginsOnly, setPluginsOnly] = useState(false);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [undo, setUndo] = useState<AgentPresetDocument | null>(null);

  const moduleByKey = useMemo(
    () => new Map(catalog.modules.map((module) => [capabilityReferenceKey(module.module), module])),
    [catalog.modules]
  );
  const selectionByKey = useMemo(
    () => new Map(document.enabled_capabilities.map((selection) => [
      capabilityReferenceKey(selection.capability),
      selection,
    ])),
    [document.enabled_capabilities]
  );
  const selectedReferences = useMemo(() => selectedModuleReferences(document), [document]);
  const selectedKeys = useMemo(
    () => new Set(selectedReferences.map(capabilityReferenceKey)),
    [selectedReferences]
  );

  const entries = useMemo<ModuleEntry[]>(() => {
    const direct: ModuleEntry[] = catalog.modules
      .filter((module) => module.authoring_policy === 'direct')
      .map((module) => ({
        module,
        missing: false,
        savedActions: selectionByKey.get(capabilityReferenceKey(module.module))?.action_allowlist ?? [],
      }));
    const visible = new Set(
      catalog.modules
        .filter((module) => module.authoring_policy === 'direct')
        .map((module) => capabilityReferenceKey(module.module))
    );
    for (const selection of document.enabled_capabilities) {
      const key = capabilityReferenceKey(selection.capability);
      if (visible.has(key)) continue;
      const catalogModule = moduleByKey.get(key);
      if (catalogModule) {
        direct.push({
          module: catalogModule,
          missing: false,
          savedActions: selection.action_allowlist ?? [],
        });
      } else {
        direct.push({
          module: selection.capability,
          display_name: String(selection.capability.id),
          description: t('agentSettings.workbench.missingReason'),
          missing: true,
          savedActions: selection.action_allowlist ?? [],
        });
      }
    }
    return direct.sort((left, right) => {
      const leftRef = left.missing ? left.module : left.module.module;
      const rightRef = right.missing ? right.module : right.module.module;
      return MODULE_CATEGORIES.indexOf(moduleCategory(leftRef)) -
        MODULE_CATEGORIES.indexOf(moduleCategory(rightRef)) ||
        String(leftRef.id).localeCompare(String(rightRef.id));
    });
  }, [catalog.modules, document.enabled_capabilities, moduleByKey, selectionByKey, t]);

  useEffect(() => {
    if (searchParams.get('source') !== 'plugin') return;
    setQuery(searchParams.get('capability') ?? '');
    setCategory('all');
    setPluginsOnly(true);
    const next = new URLSearchParams(searchParams);
    next.delete('source');
    next.delete('capability');
    setSearchParams(next, { replace: true });
  }, [searchParams, setSearchParams]);

  const referenceOf = (entry: ModuleEntry): ModuleReference =>
    entry.missing ? entry.module : entry.module.module;
  const copyOf = (entry: ModuleEntry): { name: string; description: string } => {
    if (entry.missing) return { name: entry.display_name, description: entry.description };
    const key = moduleI18nKey(String(entry.module.module.id));
    if (!key) return { name: entry.module.display_name, description: entry.module.description };
    return {
      name: t(`agentSettings.modules.${key}.name`, { defaultValue: entry.module.display_name }),
      description: t(`agentSettings.modules.${key}.description`, {
        defaultValue: entry.module.description,
      }),
    };
  };
  const materialized = (entry: ModuleEntry): boolean =>
    !entry.missing && moduleIsAvailable(entry.module, catalog.capabilities);
  const selectable = (entry: ModuleEntry): boolean =>
    !entry.missing && moduleIsSelectable(entry.module, catalog.capabilities);
  const hasUnknownActions = (entry: ModuleEntry): boolean => {
    if (entry.missing) return entry.savedActions.length > 0;
    const actions = new Set(entry.module.actions.map((action) => action.action_id));
    return entry.savedActions.some((action) => !actions.has(action));
  };
  const hasEmptyActionGrant = (entry: ModuleEntry): boolean =>
    !entry.missing && entry.module.actions.length > 0 && entry.savedActions.length === 0;
  const needsAttention = (entry: ModuleEntry): boolean => {
    const key = capabilityReferenceKey(referenceOf(entry));
    return selectedKeys.has(key) && (
      !materialized(entry) || hasUnknownActions(entry) || hasEmptyActionGrant(entry)
    );
  };
  const localizedActionName = (actionId: string): string => t(
    `agentSettings.actionLabels.${actionTranslationKey(actionId)}`,
    { defaultValue: actionFallbackName(actionId) }
  );
  const resourceName = (value: string): string => {
    const key = RESOURCE_KIND_I18N_KEYS[value];
    return key ? t(`agentSettings.resources.kinds.${key}`) : humanizeResourceKind(value);
  };
  const unavailableGuidance = (code: string | undefined): string => {
    const normalized = (code ?? 'unknown').toLocaleLowerCase();
    return t(`agentSettings.availability.${normalized}`, {
      defaultValue: t('agentSettings.workbench.moduleUnavailable'),
    });
  };

  const visibleEntries = entries.filter((entry) => {
    const reference = referenceOf(entry);
    const key = capabilityReferenceKey(reference);
    const copy = copyOf(entry);
    const matchesQuery = entry.missing
      ? `${copy.name} ${copy.description} ${reference.id} ${entry.savedActions.join(' ')}`
        .toLocaleLowerCase().includes(query.trim().toLocaleLowerCase())
      : moduleMatchesSearch(entry.module, query, copy.name, copy.description);
    return (category === 'all' || moduleCategory(reference) === category) &&
      (!pluginsOnly || (!entry.missing && !isBuiltinModule(entry.module, catalog.capabilities))) &&
      (status === 'all' || (status === 'enabled' && selectedKeys.has(key)) ||
        (status === 'attention' && needsAttention(entry))) && matchesQuery;
  });

  const changeModule = (entry: ModuleEntry, enable: boolean) => {
    if (disabled) return;
    const reference = referenceOf(entry);
    const plan = planModuleChange(document, catalog, [reference], enable);
    const names = (references: ModuleReference[]) => references.map((candidate) => {
      const found = entries.find((value) =>
        capabilityReferenceKey(referenceOf(value)) === capabilityReferenceKey(candidate)
      );
      return found ? copyOf(found).name : String(candidate.id);
    }).join('、');
    if (plan.blocked.length) {
      Modal.error({
        title: t('agentSettings.workbench.changeBlocked'),
        content: t('agentSettings.workbench.changeBlockedBody', { names: names(plan.blocked) }),
      });
      return;
    }
    const apply = () => {
      setUndo(document);
      onChange(plan.document);
      if (enable) {
        setExpanded((current) => new Set(current).add(capabilityReferenceKey(reference)));
      }
    };
    if (plan.additional.length) {
      Modal.confirm({
        title: t('agentSettings.workbench.relatedChanges'),
        content: t(
          enable ? 'agentSettings.workbench.enableDependencies' : 'agentSettings.workbench.disableDependents',
          { names: names(plan.additional) }
        ),
        okText: t('common.confirm'),
        cancelText: t('common.cancel'),
        onOk: apply,
      });
    } else apply();
  };

  const toggleAction = (entry: ModuleEntry, actionId: string) => {
    const reference = referenceOf(entry);
    const current = new Set(entry.savedActions);
    if (current.has(actionId)) current.delete(actionId);
    else current.add(actionId);
    setUndo(document);
    onChange(setModuleActions(document, reference, [...current]));
  };

  const selectedCount = selectedKeys.size;
  const attentionCount = entries.filter(needsAttention).length;
  const resourceModuleCount = entries.filter((entry) => {
    const key = capabilityReferenceKey(referenceOf(entry));
    return selectedKeys.has(key) && !entry.missing && entry.module.required_resource_kinds.length > 0;
  }).length;

  return (
    <div className={styles.workspace}>
      <div className={styles.overview}>
        <div className={styles.metrics} aria-live='polite'>
          <span><i />{t('agentSettings.workbench.enabledModules')}<strong>{selectedCount}</strong></span>
          <span>{t('agentSettings.workbench.resourceModules')}<strong>{resourceModuleCount}</strong></span>
          {attentionCount > 0 && <span className={styles.attentionMetric}>
            {t('agentSettings.workbench.needsAttention')}<strong>{attentionCount}</strong>
          </span>}
        </div>
        <span className={styles.guide}>{t('agentSettings.workbench.moduleGuide')}</span>
        {undo && <Button type='text' size='mini' disabled={disabled} onClick={() => {
          onChange(undo);
          setUndo(null);
        }}>{t('agentSettings.workbench.undo')}</Button>}
      </div>

      {attentionCount > 0 && (
        <div className={styles.notice} role='status'>
          <Info theme='outline' size={16} />
          <span>{t('agentSettings.workbench.moduleUnavailableSummary', { count: attentionCount })}</span>
        </div>
      )}

      <div className={styles.toolbar}>
        <label className={styles.searchField}>
          <Search theme='outline' size={15} />
          <input
            type='search'
            value={query}
            placeholder={t('agentSettings.workbench.searchModules')}
            aria-label={t('agentSettings.workbench.searchModules')}
            onInput={(event) => setQuery(event.currentTarget.value)}
          />
        </label>
        <div className={styles.statusFilters} role='group' aria-label={t('agentSettings.workbench.filterStatus')}>
          {(['all', 'enabled', 'attention'] as const).map((value) => (
            <button
              type='button'
              key={value}
              aria-pressed={status === value}
              onClick={() => setStatus(value)}
            >
              {t(`agentSettings.workbench.moduleStatus.${value}`)}
            </button>
          ))}
        </div>
        {pluginsOnly && <button type='button' className={styles.sourceChip} onClick={() => setPluginsOnly(false)}>
          {t('agentSettings.workbench.plugin')} · {t('agentSettings.workbench.clearFilters')}
        </button>}
      </div>

      <div className={styles.catalogLayout}>
        <nav className={styles.categories} aria-label={t('agentSettings.workbench.moduleCategories')}>
          <button type='button' aria-pressed={category === 'all'} onClick={() => setCategory('all')}>
            <Connection theme='outline' size={15} />
            <span>{t('agentSettings.workbench.allCategories')}</span>
            <small>{entries.length}</small>
          </button>
          {MODULE_CATEGORIES.map((value) => (
            <button
              type='button'
              key={value}
              aria-pressed={category === value}
              onClick={() => setCategory(value)}
            >
              <ModuleCategoryIcon category={value} size={15} />
              <span>{t(`agentSettings.workbench.categories.${value}`)}</span>
              <small>{entries.filter((entry) => moduleCategory(referenceOf(entry)) === value).length}</small>
            </button>
          ))}
        </nav>

        <section className={styles.moduleResults} aria-label={t('agentSettings.workbench.moduleCatalog')}>
          <div className={styles.resultHeading}>
            <span>{t('agentSettings.workbench.results', { count: visibleEntries.length })}</span>
            {(query || category !== 'all' || status !== 'all' || pluginsOnly) && (
              <button type='button' onClick={() => {
                setQuery('');
                setCategory('all');
                setStatus('all');
                setPluginsOnly(false);
              }}>{t('agentSettings.workbench.clearFilters')}</button>
            )}
          </div>

          <div className={styles.moduleGrid}>
            {visibleEntries.map((entry) => {
              const reference = referenceOf(entry);
              const key = capabilityReferenceKey(reference);
              const selected = selectedKeys.has(key);
              const canEnable = selectable(entry);
              const isMaterialized = materialized(entry);
              const copy = copyOf(entry);
              const open = expanded.has(key);
              const availability = entry.missing ? undefined : moduleAvailability(entry.module, catalog.capabilities);
              const knownActions: CapabilityModuleAction[] = entry.missing ? [] : entry.module.actions;
              const knownActionIds = new Set(knownActions.map((action) => action.action_id));
              const missingActions = entry.savedActions.filter((action) => !knownActionIds.has(action));
              const resources = entry.missing ? [] : entry.module.required_resource_kinds;
              return (
                <article
                  key={key}
                  className={`${styles.moduleCard} ${selected ? styles.moduleCardEnabled : ''} ${needsAttention(entry) ? styles.moduleCardAttention : ''}`}
                >
                  <div className={styles.moduleHeader}>
                    <span className={styles.moduleIcon}>
                      <ModuleCategoryIcon category={moduleCategory(reference)} size={18} />
                    </span>
                    <div className={styles.moduleCopy}>
                      <div className={styles.moduleTitleLine}>
                        <h3>{copy.name}</h3>
                        {!entry.missing && !isBuiltinModule(entry.module, catalog.capabilities) && <Tag size='small'>{t('agentSettings.workbench.plugin')}</Tag>}
                      </div>
                      <p>{copy.description}</p>
                    </div>
                    <button
                      type='button'
                      role='switch'
                      aria-checked={selected}
                      aria-label={t(selected ? 'agentSettings.workbench.disableModule' : 'agentSettings.workbench.enableModule', { name: copy.name })}
                      className={styles.moduleSwitch}
                      disabled={disabled || (!selected && !canEnable)}
                      onClick={() => changeModule(entry, !selected)}
                    >
                      <span />
                    </button>
                  </div>

                  <div className={styles.moduleMeta}>
                    <span className={selected ? styles.enabledState : styles.disabledState}>
                      {selected && <Check theme='outline' size={12} />}
                      {t(selected ? 'agentSettings.capabilities.enabled' : 'agentSettings.capabilities.notSelected')}
                    </span>
                    {!isMaterialized && !selected && <span className={styles.unavailableState}>
                      {t('agentSettings.common.unavailable')}
                    </span>}
                    {resources.length > 0 ? (
                      <span className={styles.resourceState}>
                        {t('agentSettings.resources.requiredCount', { count: resources.length })}
                      </span>
                    ) : (
                      <span className={styles.resourceState}>{t('agentSettings.resources.noneRequired')}</span>
                    )}
                  </div>

                  {needsAttention(entry) && (
                    <div className={styles.cardWarning} role='status'>
                      <Info theme='outline' size={14} />
                      <span>{entry.missing
                        ? t('agentSettings.workbench.missingReason')
                        : missingActions.length
                          ? t('agentSettings.workbench.actionMissing')
                          : hasEmptyActionGrant(entry)
                            ? t('agentSettings.workbench.actionRequired')
                          : unavailableGuidance(availability?.unavailable_code)}</span>
                    </div>
                  )}

                  <button
                    type='button'
                    className={styles.disclosure}
                    aria-expanded={open}
                    aria-controls={`module-detail-${key.replace(/[^a-zA-Z0-9]/g, '-')}`}
                    onClick={() => setExpanded((current) => {
                      const next = new Set(current);
                      if (next.has(key)) next.delete(key);
                      else next.add(key);
                      return next;
                    })}
                  >
                    <span>{t('agentSettings.workbench.actionCount', { count: knownActions.length + missingActions.length })}</span>
                    <Down theme='outline' size={14} className={open ? styles.disclosureOpen : ''} />
                  </button>

                  {open && (
                    <div className={styles.moduleDetail} id={`module-detail-${key.replace(/[^a-zA-Z0-9]/g, '-')}`}>
                      {resources.length > 0 && (
                        <div className={styles.resourcePanel}>
                          <strong>{t('agentSettings.resources.bindingStatus')}</strong>
                          <p>{t('agentSettings.resources.bindingStatusHint')}</p>
                          <div>{resources.map((resource) => <Tag key={resource}>{resourceName(resource)}</Tag>)}</div>
                        </div>
                      )}
                      <fieldset className={styles.actionList} disabled={disabled || !selected}>
                        <legend>{t('agentSettings.workbench.actionPermissions')}</legend>
                        {knownActions.map((action) => {
                          const actionName = localizedActionName(action.action_id);
                          return (
                            <div key={action.action_id} className={styles.actionRow}>
                              <Checkbox
                                checked={entry.savedActions.includes(action.action_id)}
                                onChange={() => toggleAction(entry, action.action_id)}
                                aria-label={t('agentSettings.workbench.toggleAction', { action: actionName, module: copy.name })}
                              />
                              <span className={styles.actionCopy}>
                                <strong>{actionName}</strong>
                                <small className={styles[`effect_${effectTone(action.effect_class)}`]}>
                                  {t(`agentSettings.effects.${action.effect_class}`, { defaultValue: action.effect_class })}
                                </small>
                              </span>
                              <details className={styles.actionTechnical}>
                                <summary>{t('common.technical_details')}</summary>
                                <dl>
                                  <dt>{t('agentSettings.workbench.actionId')}</dt><dd><code>{action.action_id}</code></dd>
                                  <dt>{t('agentSettings.workbench.inputContract')}</dt><dd><code>{action.input_schema}</code></dd>
                                  <dt>{t('agentSettings.workbench.outputContract')}</dt><dd><code>{action.output_schema}</code></dd>
                                </dl>
                              </details>
                            </div>
                          );
                        })}
                        {missingActions.map((actionId) => (
                          <div key={actionId} className={`${styles.actionRow} ${styles.actionMissing}`}>
                            <Checkbox
                              checked
                              onChange={() => toggleAction(entry, actionId)}
                              aria-label={t('agentSettings.workbench.removeMissingAction', { action: actionId })}
                            />
                            <span className={styles.actionCopy}>
                              <strong>{actionFallbackName(actionId)}</strong>
                              <small>{t('agentSettings.workbench.actionUnavailable')}</small>
                            </span>
                            <code>{actionId}</code>
                          </div>
                        ))}
                        {knownActions.length === 0 && missingActions.length === 0 && (
                          <p className={styles.noActions}>{t('agentSettings.workbench.contextOnlyModule')}</p>
                        )}
                      </fieldset>
                      <details className={styles.technicalDetail}>
                        <summary>{t('agentSettings.workbench.moduleTechnicalDetails')}</summary>
                        <dl>
                          <dt>ID</dt><dd>{reference.id}</dd>
                          <dt>{t('agentSettings.workbench.version')}</dt><dd>{reference.version}</dd>
                          <dt>{t('agentSettings.capabilities.source')}</dt>
                          <dd>{entry.missing ? '—' : entry.module.source_package.id}</dd>
                          {availability?.unavailable_code && <>
                            <dt>{t('agentSettings.workbench.availabilityCode')}</dt>
                            <dd><code>{availability.unavailable_code}</code></dd>
                          </>}
                        </dl>
                      </details>
                    </div>
                  )}
                </article>
              );
            })}
          </div>

          {visibleEntries.length === 0 && (
            <div className={styles.empty} role='status'>
              <Connection theme='outline' size={30} />
              <h3>{t(entries.length ? 'agentSettings.workbench.noModuleResults' : 'agentSettings.workbench.noModules')}</h3>
              <p>{t(entries.length ? 'agentSettings.workbench.noResultsHint' : 'agentSettings.workbench.noModulesHint')}</p>
              {entries.length > 0 && <Button size='small' icon={<Refresh />} onClick={() => {
                setQuery('');
                setCategory('all');
                setStatus('all');
                setPluginsOnly(false);
              }}>{t('agentSettings.workbench.clearFilters')}</Button>}
            </div>
          )}
        </section>
      </div>
    </div>
  );
};

export default AgentCapabilityWorkspace;
