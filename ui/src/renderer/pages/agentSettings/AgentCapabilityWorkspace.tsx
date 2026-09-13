import type { AgentPresetDocument, CapabilityCatalogItem } from '@/common/types/agentPlatform';
import { Button, Checkbox, Message, Modal } from '@arco-design/web-react';
import { ArrowLeft, ArrowRight, Add, Minus, Book, Code, Connection, Earth, Info, Lightning, Magic, Search, User, Check } from '@icon-park/react';
import React, { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useSearchParams } from 'react-router-dom';
import { CAPABILITY_CATEGORIES, capabilityCategory, capabilityIsAvailable, isBuiltinCapability, selectedCapabilityReferences, type CapabilityCategory, type CapabilityReference } from './capabilityGroups';
import { capabilityMatchesSearch, capabilityProductCopy, capabilityProductName, capabilityReferenceKey, humanizeResourceKind, RESOURCE_KIND_I18N_KEYS } from './model';
import { planCapabilityChange } from './capabilityChanges';
import styles from './AgentCapabilityWorkspace.module.css';

const categoryIcons = { knowledge: Book, development: Code, web: Earth, collaboration: User, creation: Magic, automation: Lightning, models: Connection, integrations: Connection };
export const CapabilityCategoryIcon: React.FC<{ category: CapabilityCategory; size?: number }> = ({ category, size = 16 }) => {
  const Icon = categoryIcons[category];
  return <Icon theme='outline' size={size} />;
};
type Entry = { reference: CapabilityReference; item?: CapabilityCatalogItem; name: string; description: string };
type PaneProps = {
  side: 'enabled' | 'catalog'; entries: Entry[]; selectedKeys: Set<string>;
  checked: Set<string>; onChecked: (checked: Set<string>) => void; disabled: boolean;
  onChange: (references: CapabilityReference[], enabled: boolean) => void;
  onDetails: (reference: CapabilityReference) => void; pluginSearch?: string;
};

const CapabilityPane: React.FC<PaneProps> = ({ side, entries, selectedKeys, checked, onChecked, disabled, onChange, onDetails, pluginSearch }) => {
  const { t, i18n } = useTranslation();
  const left = side === 'enabled';
  const [category, setCategory] = useState<CapabilityCategory | 'all'>('all');
  const [search, setSearch] = useState('');
  const [onlyDisabled, setOnlyDisabled] = useState(false);
  const [pluginsOnly, setPluginsOnly] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (pluginSearch === undefined) return;
    setSearch(pluginSearch); setCategory('all'); setPluginsOnly(true); setOnlyDisabled(false);
  }, [pluginSearch]);
  const filtered = entries.filter(entry =>
    (category === 'all' || capabilityCategory(entry.reference) === category) &&
    (!onlyDisabled || !selectedKeys.has(capabilityReferenceKey(entry.reference))) &&
    (!pluginsOnly || (entry.item && !isBuiltinCapability(entry.item))) &&
    (entry.item ? capabilityMatchesSearch(entry.item, search, i18n.language) : `${entry.name} ${entry.reference.id}`.toLowerCase().includes(search.trim().toLowerCase())),
  );
  const selectable = filtered.filter(entry => left || (!selectedKeys.has(capabilityReferenceKey(entry.reference)) && capabilityIsAvailable(entry.item)));
  const allChecked = selectable.length > 0 && selectable.every(entry => checked.has(capabilityReferenceKey(entry.reference)));
  const someChecked = selectable.some(entry => checked.has(capabilityReferenceKey(entry.reference)));
  const filter = (action: () => void) => { action(); onChecked(new Set()); scrollRef.current?.scrollTo?.(0, 0); };
  const clearFilters = () => filter(() => { setSearch(''); setCategory('all'); setOnlyDisabled(false); setPluginsOnly(false); });
  return <section className={styles.pane} aria-label={t(left ? 'agentSettings.workbench.enabledCapabilities' : 'agentSettings.workbench.allCapabilities')}>
    <div className={styles.paneHeading}><h3>{t(left ? 'agentSettings.workbench.enabledCapabilities' : 'agentSettings.workbench.allCapabilities')}<span className={left ? styles.enabledCount : styles.count}>{entries.length}</span></h3><span>{t(left ? 'agentSettings.workbench.enabledHint' : 'agentSettings.workbench.catalogHint')}</span></div>
    <label className={styles.searchField}><Search theme='outline' size={15} /><input type='search' value={search} placeholder={t(left ? 'agentSettings.workbench.searchEnabled' : 'agentSettings.workbench.searchLibrary')} aria-label={t(left ? 'agentSettings.workbench.searchEnabled' : 'agentSettings.workbench.searchLibrary')} onChange={event => filter(() => setSearch(event.target.value))} /></label>
    {pluginsOnly && <div className={styles.sourceFilter}>{t('agentSettings.workbench.plugin')}<button type='button' onClick={() => filter(() => setPluginsOnly(false))}>{t('agentSettings.workbench.allSources')}</button></div>}
    <div className={styles.paneBody}>
      <nav className={styles.categories} aria-label={t(left ? 'agentSettings.workbench.enabledCategories' : 'agentSettings.workbench.libraryCategories')}>
        <span className={styles.categoryLabel}>{t('agentSettings.workbench.category')}</span>
        <button type='button' aria-pressed={category === 'all'} onClick={() => filter(() => setCategory('all'))}><Connection theme='outline' size={14} /><span>{t('agentSettings.workbench.allCategories')}</span><small>{entries.length}</small></button>
        {CAPABILITY_CATEGORIES.map(key => <button type='button' key={key} aria-pressed={category === key} onClick={() => filter(() => setCategory(key))}><CapabilityCategoryIcon category={key} size={14} /><span>{t(`agentSettings.workbench.categories.${key}`)}</span><small>{entries.filter(entry => capabilityCategory(entry.reference) === key).length}</small></button>)}
      </nav>
      <div className={styles.resultColumn}>
        <div className={styles.resultToolbar}><Checkbox checked={allChecked} indeterminate={!allChecked && someChecked} disabled={disabled || selectable.length === 0} aria-label={t(left ? 'agentSettings.workbench.selectEnabledResults' : 'agentSettings.workbench.selectAvailableResults')} onChange={() => onChecked(allChecked ? new Set() : new Set(selectable.map(entry => capabilityReferenceKey(entry.reference))))}>{t('agentSettings.workbench.selectResults')}</Checkbox>{!left && <Checkbox checked={onlyDisabled} onChange={value => filter(() => setOnlyDisabled(value))}>{t('agentSettings.workbench.onlyDisabled')}</Checkbox>}<span>{t('agentSettings.workbench.results', { count: filtered.length })}</span></div>
        <div className={styles.rows} ref={scrollRef}>
          {filtered.map(entry => {
            const key = capabilityReferenceKey(entry.reference), enabled = selectedKeys.has(key), available = capabilityIsAvailable(entry.item);
            const cannotSelect = disabled || (!left && (enabled || !available));
            return <div key={key} className={`${styles.row} ${checked.has(key) ? styles.rowChecked : ''}`}>
              <Checkbox checked={checked.has(key)} disabled={cannotSelect} aria-label={t(left ? 'agentSettings.workbench.selectCapability' : 'agentSettings.workbench.addCapability', { name: entry.name })} onChange={value => { const next = new Set(checked); if (value) next.add(key); else next.delete(key); onChecked(next); }} />
              <button type='button' className={styles.rowCopy} onClick={() => onDetails(entry.reference)} aria-label={t('agentSettings.workbench.detailsAria', { name: entry.name })}><strong>{entry.name}</strong><span>{entry.description}</span>{!available && <small className={styles.unavailable}>{t('agentSettings.workbench.unavailableDetails')}</small>}</button>
              <div className={styles.rowActions}>{!left && <span className={enabled ? styles.enabledState : styles.disabledState}>{enabled && <Check theme='outline' size={12} />}{t(enabled ? 'agentSettings.capabilities.enabled' : 'agentSettings.capabilities.notSelected')}</span>}<button type='button' className={styles.quickAction} disabled={cannotSelect} onClick={() => onChange([entry.reference], !left)} title={t(left ? 'agentSettings.workbench.disableCapability' : 'agentSettings.workbench.enableCapability', { name: entry.name })} aria-label={t(left ? 'agentSettings.workbench.disableCapability' : 'agentSettings.workbench.enableCapability', { name: entry.name })}>{left ? <Minus theme='outline' size={15} /> : <Add theme='outline' size={15} />}</button></div>
            </div>;
          })}
          {!filtered.length && <div className={styles.empty} role='status'><Connection theme='outline' size={28} /><h3>{t(entries.length ? 'agentSettings.workbench.noResults' : 'agentSettings.workbench.emptyTitle')}</h3><p>{t(entries.length ? 'agentSettings.workbench.noResultsHint' : 'agentSettings.workbench.emptyHint')}</p>{entries.length > 0 && <Button size='small' onClick={clearFilters}>{t('agentSettings.workbench.clearFilters')}</Button>}</div>}
        </div>
      </div>
    </div>
    <div className={styles.paneFooter}><span>{t(checked.size ? 'agentSettings.workbench.selectedCount' : 'agentSettings.workbench.selectionHint', { count: checked.size })}</span>{checked.size > 0 && <button type='button' onClick={() => onChecked(new Set())}>{t('agentSettings.workbench.clearSelection')}</button>}</div>
  </section>;
};

type Props = { document: AgentPresetDocument; catalog: readonly CapabilityCatalogItem[]; disabled?: boolean; onChange: (document: AgentPresetDocument) => void };
const AgentCapabilityWorkspace: React.FC<Props> = ({ document, catalog, disabled = false, onChange }) => {
  const { t, i18n } = useTranslation();
  const [searchParams, setSearchParams] = useSearchParams();
  const [leftChecked, setLeftChecked] = useState(new Set<string>());
  const [rightChecked, setRightChecked] = useState(new Set<string>());
  const [details, setDetails] = useState<CapabilityReference | null>(null);
  const [undo, setUndo] = useState<AgentPresetDocument | null>(null);
  const [pluginSearch, setPluginSearch] = useState<string>();
  const byKey = useMemo(() => new Map(catalog.map(item => [capabilityReferenceKey(item.capability), item])), [catalog]);
  const selected = useMemo(() => selectedCapabilityReferences(document), [document]);
  const selectedKeys = useMemo(() => new Set(selected.map(capabilityReferenceKey)), [selected]);
  const entryOf = (reference: CapabilityReference): Entry => {
    const item = byKey.get(capabilityReferenceKey(reference));
    return { reference, item, ...(item ? capabilityProductCopy(item, i18n.language) : { name: capabilityProductName(reference.id, i18n.language), description: t('agentSettings.workbench.missingReason') }) };
  };
  const sort = (a: Entry, b: Entry) => CAPABILITY_CATEGORIES.indexOf(capabilityCategory(a.reference)) - CAPABILITY_CATEGORIES.indexOf(capabilityCategory(b.reference));
  const enabledEntries = selected.map(entryOf).sort(sort);
  const catalogEntries = catalog.map(item => entryOf(item.capability)).sort((a, b) => sort(a, b) || Number(capabilityIsAvailable(b.item)) - Number(capabilityIsAvailable(a.item)));
  const unavailable = enabledEntries.filter(entry => !capabilityIsAvailable(entry.item));
  useEffect(() => {
    setLeftChecked(current => new Set([...current].filter(key => selectedKeys.has(key))));
    setRightChecked(current => new Set([...current].filter(key => !selectedKeys.has(key) && capabilityIsAvailable(byKey.get(key)))));
  }, [selectedKeys, byKey]);
  useEffect(() => {
    if (searchParams.get('source') !== 'plugin') return;
    setPluginSearch(searchParams.get('capability') ?? '');
    const next = new URLSearchParams(searchParams); next.delete('source'); next.delete('capability');
    setSearchParams(next, { replace: true });
  }, [searchParams, setSearchParams]);
  const change = (references: CapabilityReference[], enable: boolean) => {
    if (disabled) return;
    const plan = planCapabilityChange(document, catalog, references, enable);
    const names = (items: CapabilityReference[]) => items.map(reference => entryOf(reference).name).join('、');
    if (plan.blocked.length) { Modal.error({ title: t('agentSettings.workbench.changeBlocked'), content: t('agentSettings.workbench.changeBlockedBody', { names: names(plan.blocked) }) }); return; }
    const apply = () => { setUndo(document); onChange(plan.document); setLeftChecked(new Set()); setRightChecked(new Set()); Message.success(t(enable ? 'agentSettings.workbench.enabledToast' : 'agentSettings.workbench.disabledToast', { count: plan.affected.length })); };
    if (!plan.affected.length) return;
    if (plan.additional.length) Modal.confirm({ title: t('agentSettings.workbench.relatedChanges'), content: t(enable ? 'agentSettings.workbench.enableDependencies' : 'agentSettings.workbench.disableDependents', { names: names(plan.additional) }), okText: t('common.confirm'), cancelText: t('common.cancel'), onOk: apply });
    else apply();
  };
  const resourceName = (value: string) => { const key = RESOURCE_KIND_I18N_KEYS[value]; return key ? t(`agentSettings.resources.kinds.${key}`) : humanizeResourceKind(value); };
  const detail = details ? entryOf(details) : null;
  return <div className={styles.workspace}>
    <div className={styles.overview}><div className={styles.metrics}><span><i />{t('agentSettings.capabilities.enabled')}<strong>{selected.length}</strong></span><span>{t('agentSettings.capabilities.notSelected')}<strong>{catalogEntries.filter(entry => !selectedKeys.has(capabilityReferenceKey(entry.reference))).length}</strong></span></div><span className={styles.guide}>{t('agentSettings.workbench.transferHint')}</span>{undo && <Button type='text' size='mini' disabled={disabled} onClick={() => { onChange(undo); setUndo(null); setLeftChecked(new Set()); setRightChecked(new Set()); }}>{t('agentSettings.workbench.undo')}</Button>}</div>
    {unavailable.length > 0 && <div className={styles.notice} role='status'><Info theme='outline' size={16} /><span>{t('agentSettings.workbench.unavailableSummary', { count: unavailable.length })}</span><Button size='mini' type='text' disabled={disabled} onClick={() => change(unavailable.map(entry => entry.reference), false)}>{t('agentSettings.workbench.removeUnavailable')}</Button></div>}
    <div className={styles.transfer}>
      <CapabilityPane side='enabled' entries={enabledEntries} selectedKeys={selectedKeys} checked={leftChecked} onChecked={setLeftChecked} disabled={disabled} onChange={change} onDetails={setDetails} />
      <div className={styles.transferActions}><button type='button' className={styles.moveIn} disabled={disabled || !rightChecked.size} onClick={() => change(catalogEntries.filter(entry => rightChecked.has(capabilityReferenceKey(entry.reference))).map(entry => entry.reference), true)}><ArrowLeft theme='outline' size={19} /><span>{t('agentSettings.workbench.moveIn')}{rightChecked.size > 0 && ` (${rightChecked.size})`}</span></button><button type='button' disabled={disabled || !leftChecked.size} onClick={() => change(selected.filter(reference => leftChecked.has(capabilityReferenceKey(reference))), false)}><ArrowRight theme='outline' size={19} /><span>{t('agentSettings.workbench.moveOut')}{leftChecked.size > 0 && ` (${leftChecked.size})`}</span></button><small>{t('agentSettings.workbench.multiSelect')}</small></div>
      <CapabilityPane side='catalog' entries={catalogEntries} selectedKeys={selectedKeys} checked={rightChecked} onChecked={setRightChecked} disabled={disabled} onChange={change} onDetails={setDetails} pluginSearch={pluginSearch} />
    </div>
    <Modal autoFocus focusLock title={detail?.name ?? ''} visible={detail !== null} style={{ width: 'min(440px, calc(100vw - 24px))' }} onCancel={() => setDetails(null)} unmountOnExit footer={detail && <Button type='primary' disabled={disabled || (!selectedKeys.has(capabilityReferenceKey(detail.reference)) && !capabilityIsAvailable(detail.item))} onClick={() => { change([detail.reference], !selectedKeys.has(capabilityReferenceKey(detail.reference))); setDetails(null); }}>{t(selectedKeys.has(capabilityReferenceKey(detail.reference)) ? 'agentSettings.workbench.moveOut' : 'agentSettings.workbench.moveIn')}</Button>}>
      {detail && <div className={styles.detail}><span className={selectedKeys.has(capabilityReferenceKey(detail.reference)) ? styles.enabledState : styles.disabledState}>{t(selectedKeys.has(capabilityReferenceKey(detail.reference)) ? 'agentSettings.capabilities.enabled' : 'agentSettings.capabilities.notSelected')}</span><p>{detail.description}</p>{!capabilityIsAvailable(detail.item) && <div className={styles.notice}>{t(detail.item ? isBuiltinCapability(detail.item) ? 'agentSettings.workbench.builtinReason' : 'agentSettings.workbench.pluginReason' : 'agentSettings.workbench.missingReason')}</div>}<dl><dt>{t('agentSettings.workbench.category')}</dt><dd>{t(`agentSettings.workbench.categories.${capabilityCategory(detail.reference)}`)}</dd><dt>{t('agentSettings.capabilities.source')}</dt><dd>{t(detail.item && isBuiltinCapability(detail.item) ? 'agentSettings.workbench.builtin' : 'agentSettings.workbench.plugin')}</dd><dt>{t('agentSettings.resources.requiredAtUse')}</dt><dd>{detail.item?.required_resource_kinds.length ? detail.item.required_resource_kinds.map(resourceName).join('、') : t('agentSettings.resources.noneRequired')}</dd></dl><details><summary>{t('common.technical_details')}</summary><dl><dt>ID</dt><dd>{detail.reference.id}</dd><dt>{t('agentSettings.workbench.version')}</dt><dd>{detail.reference.version}</dd><dt>{t('agentSettings.capabilities.source')}</dt><dd>{detail.item?.source_package.id ?? '—'}</dd>{detail.item?.unavailable_code && <><dt>{t('agentSettings.workbench.unavailableCode')}</dt><dd>{detail.item.unavailable_code}</dd></>}</dl></details></div>}
    </Modal>
  </div>;
};
export default AgentCapabilityWorkspace;
