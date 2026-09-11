import type { AgentPresetDocument, CapabilityCatalogItem, CapabilityPlacement } from '@/common/types/agentPlatform';
import { Button, Checkbox, Drawer, Pagination, Select, Tag } from '@arco-design/web-react';
import { Add, Book, Code, Connection, Delete, Earth, Info, Lightning, Magic, Search, User } from '@icon-park/react';
import React, { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useSearchParams } from 'react-router-dom';
import {
  CAPABILITY_CATEGORIES, capabilityCategory, capabilityIsAvailable, isBuiltinCapability,
  selectedCapabilityReferences, unavailableCapabilityReferences,
  type CapabilityCategory, type CapabilityReference,
} from './capabilityGroups';
import {
  capabilityMatchesSearch, capabilityPlacement, capabilityProductCopy, capabilityProductName,
  capabilityReferenceKey, humanizeResourceKind, placeCapability, RESOURCE_KIND_I18N_KEYS,
} from './model';
import styles from './AgentCapabilityWorkspace.module.css';

const PAGE_SIZE = 12;
const categoryIcons = { knowledge: Book, development: Code, web: Earth, collaboration: User,
  creation: Magic, automation: Lightning, models: Connection, integrations: Connection };

export const CapabilityCategoryIcon: React.FC<{ category: CapabilityCategory; size?: number }> = ({ category, size = 18 }) => {
  const Icon = categoryIcons[category];
  return <Icon theme='outline' size={size} />;
};

type Props = {
  document: AgentPresetDocument;
  catalog: readonly CapabilityCatalogItem[];
  disabled?: boolean;
  onChange: (document: AgentPresetDocument) => void;
};

const AgentCapabilityWorkspace: React.FC<Props> = ({ document, catalog, disabled = false, onChange }) => {
  const { t, i18n } = useTranslation();
  const [searchParams, setSearchParams] = useSearchParams();
  const handledPluginLink = useRef('');
  const [search, setSearch] = useState('');
  const [category, setCategory] = useState<CapabilityCategory | 'all'>('all');
  const [filter, setFilter] = useState('all');
  const [page, setPage] = useState(1);
  const [checked, setChecked] = useState<Set<string>>(new Set());
  const [pickerOpen, setPickerOpen] = useState(false);
  const [pickerSearch, setPickerSearch] = useState('');
  const [pickerCategory, setPickerCategory] = useState<CapabilityCategory | 'all'>('all');
  const [pickerSource, setPickerSource] = useState('all');
  const [showUnavailable, setShowUnavailable] = useState(false);
  const [pickerPage, setPickerPage] = useState(1);
  const [additions, setAdditions] = useState<Set<string>>(new Set());
  const [details, setDetails] = useState<CapabilityReference | null>(null);
  const byKey = useMemo(() => new Map(catalog.map((item) => [capabilityReferenceKey(item.capability), item])), [catalog]);
  const selected = useMemo(() => selectedCapabilityReferences(document), [document]);
  const selectedKeys = useMemo(() => new Set(selected.map(capabilityReferenceKey)), [selected]);
  const unavailable = useMemo(() => unavailableCapabilityReferences(document, catalog), [document, catalog]);
  const categoryName = (value: CapabilityCategory) => t(`agentSettings.workbench.categories.${value}`);
  const isGeneratedPluginCapability = (reference: CapabilityReference) =>
    reference.id.startsWith('user.nomifun.plugin-');
  const nameOf = (reference: CapabilityReference) => {
    const item = byKey.get(capabilityReferenceKey(reference));
    if (item) return capabilityProductCopy(item, i18n.language).name;
    if (isGeneratedPluginCapability(reference)) {
      const name = reference.id.split('.').at(-1)?.replace(/[_-]+/g, ' ') ?? reference.id;
      return t('agentSettings.workbench.pluginCapabilityFallback', { name });
    }
    return capabilityProductName(reference.id, i18n.language);
  };
  const resourceName = (value: string) => {
    const key = RESOURCE_KIND_I18N_KEYS[value];
    return key ? t(`agentSettings.resources.kinds.${key}`) : humanizeResourceKind(value);
  };
  const matchesSearch = (reference: CapabilityReference, query: string) => {
    const item = byKey.get(capabilityReferenceKey(reference));
    return item ? capabilityMatchesSearch(item, query, i18n.language) :
      `${nameOf(reference)} ${reference.id}`.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase());
  };
  const statusLabel = (item?: CapabilityCatalogItem) => {
    if (!item) return t('agentSettings.workbench.missingSource');
    if (capabilityIsAvailable(item)) return t('agentSettings.common.available');
    return t(isBuiltinCapability(item) ? 'agentSettings.workbench.notReady' : 'agentSettings.workbench.sourceUnavailable');
  };
  const statusLabelFor = (reference: CapabilityReference, item?: CapabilityCatalogItem) =>
    !item && isGeneratedPluginCapability(reference)
      ? t('agentSettings.workbench.pluginUnavailable')
      : statusLabel(item);
  const statusReasonFor = (reference: CapabilityReference, item?: CapabilityCatalogItem) => t(
    !item && isGeneratedPluginCapability(reference)
      ? 'agentSettings.workbench.pluginReason'
      : !item
        ? 'agentSettings.workbench.missingReason'
        : isBuiltinCapability(item)
          ? 'agentSettings.workbench.builtinReason'
          : 'agentSettings.workbench.pluginReason'
  );

  const filtered = selected.filter((reference) => {
    if (category !== 'all' && capabilityCategory(reference) !== category) return false;
    if (!matchesSearch(reference, search)) return false;
    if (filter === 'unavailable') return !capabilityIsAvailable(byKey.get(capabilityReferenceKey(reference)));
    return filter === 'all' || capabilityPlacement(document, reference) === filter;
  });
  const pageRows = filtered.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE);
  const checkedReferences = selected.filter((reference) => checked.has(capabilityReferenceKey(reference)));
  const anyCheckedUnavailable = checkedReferences.some((reference) => !capabilityIsAvailable(byKey.get(capabilityReferenceKey(reference))));

  useEffect(() => { setPage(1); }, [search, category, filter]);
  useEffect(() => { setPage((current) => Math.min(current, Math.max(1, Math.ceil(filtered.length / PAGE_SIZE)))); }, [filtered.length]);
  useEffect(() => { setPickerPage(1); }, [pickerSearch, pickerCategory, pickerSource, showUnavailable]);
  useEffect(() => { setChecked((current) => new Set([...current].filter((key) => selectedKeys.has(key)))); }, [selectedKeys]);
  useEffect(() => {
    if (category !== 'all' && !selected.some((reference) => capabilityCategory(reference) === category)) setCategory('all');
  }, [category, selected]);
  useEffect(() => {
    const source = searchParams.get('source');
    const capability = searchParams.get('capability') ?? '';
    const key = `${source ?? ''}:${capability}`;
    if (source !== 'plugin' || handledPluginLink.current === key) return;
    handledPluginLink.current = key;
    setAdditions(new Set());
    setPickerSearch(capability);
    setPickerCategory('all');
    setPickerSource('plugin');
    setShowUnavailable(true);
    setPickerPage(1);
    setPickerOpen(true);
    const next = new URLSearchParams(searchParams);
    next.delete('source');
    next.delete('capability');
    setSearchParams(next, { replace: true });
  }, [searchParams, setSearchParams]);

  const setMode = (references: readonly CapabilityReference[], mode: CapabilityPlacement) => {
    onChange(references.reduce((next, reference) => placeCapability(next, reference, mode), document));
    setChecked(new Set());
    if (mode === 'none' && filter === 'unavailable') setFilter('all');
  };
  const selectMode = (mode: string) => { setFilter(mode); setCategory('all'); setSearch(''); };
  const toggleChecked = (key: string, value: boolean) => setChecked((previous) => {
    const next = new Set(previous); if (value) next.add(key); else next.delete(key); return next;
  });
  const openPicker = () => { setAdditions(new Set()); setPickerSearch(''); setPickerCategory('all'); setPickerPage(1); setPickerOpen(true); };
  const candidates = catalog.filter((item) => {
    if (!showUnavailable && !capabilityIsAvailable(item)) return false;
    if (pickerSource === 'builtin' && !isBuiltinCapability(item)) return false;
    if (pickerSource === 'plugin' && isBuiltinCapability(item)) return false;
    return capabilityMatchesSearch(item, pickerSearch, i18n.language);
  });
  const pickerFiltered = candidates.filter((item) => pickerCategory === 'all' || capabilityCategory(item.capability) === pickerCategory);
  useEffect(() => { setPickerPage((current) => Math.min(current, Math.max(1, Math.ceil(pickerFiltered.length / PAGE_SIZE)))); }, [pickerFiltered.length]);
  const pickerRows = pickerFiltered.slice((pickerPage - 1) * PAGE_SIZE, pickerPage * PAGE_SIZE);
  const applyAdditions = () => {
    const references = catalog.filter((item) => additions.has(capabilityReferenceKey(item.capability)) && capabilityIsAvailable(item)).map((item) => item.capability);
    setMode(references, 'on_demand');
    setCategory('all'); setFilter('all'); setSearch(''); setPage(1);
    setPickerOpen(false);
  };

  const renderDetails = (reference: CapabilityReference) => {
    const item = byKey.get(capabilityReferenceKey(reference));
    return <div className={styles.detailContent}>
      <p>{item ? capabilityProductCopy(item, i18n.language).description : statusReasonFor(reference, item)}</p>
      {!capabilityIsAvailable(item) && <div className={styles.detailWarning}>{statusReasonFor(reference, item)}</div>}
      <dl className={styles.detailFacts}>
        <div><dt>{t('agentSettings.workbench.category')}</dt><dd>{categoryName(capabilityCategory(reference))}</dd></div>
        <div><dt>{t('agentSettings.capabilities.source')}</dt><dd>{item ? t(isBuiltinCapability(item) ? 'agentSettings.workbench.builtin' : 'agentSettings.workbench.plugin') : statusLabelFor(reference, item)}</dd></div>
        <div><dt>{t('agentSettings.resources.requiredAtUse')}</dt><dd>{item?.required_resource_kinds.length ? item.required_resource_kinds.map(resourceName).join('、') : t('agentSettings.resources.noneRequired')}</dd></div>
      </dl>
      <details className={styles.technicalDetails}><summary>{t('common.technical_details')}</summary>
        <dl className={styles.detailFacts}>
          <div><dt>ID</dt><dd><code>{reference.id}</code></dd></div>
          <div><dt>{t('agentSettings.workbench.version')}</dt><dd>{reference.version}</dd></div>
          {item && <><div><dt>{t('agentSettings.capabilities.source')}</dt><dd><code>{item.source_package.id}@{item.source_package.version}</code></dd></div>
            <div><dt>{t('agentSettings.workbench.contributions')}</dt><dd>{t('agentSettings.capabilities.tools', { count: item.action_count })} · {t('agentSettings.capabilities.contexts', { count: item.context_contributor_count })}</dd></div></>}
        </dl>
      </details>
    </div>;
  };

  return <div className={styles.workspace}>
    <div className={styles.overview}>
      <button type='button' className={styles.metric} onClick={() => selectMode('all')}><span>{t('agentSettings.workbench.configured')}</span><strong>{selected.length}</strong></button>
      <button type='button' className={styles.metric} onClick={() => selectMode('initial')}><span>{t('agentSettings.capabilities.initialShort')}</span><strong>{document.initial_capabilities.length}</strong></button>
      <button type='button' className={styles.metric} onClick={() => selectMode('on_demand')}><span>{t('agentSettings.capabilities.onDemandShort')}</span><strong>{document.on_demand_capabilities.length}</strong></button>
      <button type='button' className={`${styles.metric} ${unavailable.length ? styles.metricWarning : ''}`} onClick={() => selectMode('unavailable')}><span>{t('agentSettings.workbench.needsAttention')}</span><strong>{unavailable.length}</strong></button>
    </div>

    {unavailable.length > 0 && filter === 'unavailable' && <div className={styles.notice} role='status'><Info theme='outline' size={18} /><span>{t('agentSettings.workbench.unavailableSummary', { count: unavailable.length })}</span>
      <Button size='mini' type='text' disabled={disabled} onClick={() => setMode(unavailable, 'none')}>{t('agentSettings.workbench.removeUnavailable')}</Button>
    </div>}

    <div className={styles.toolbar}>
      <label className={styles.searchField}><Search theme='outline' size={16} /><input type='search' value={search} placeholder={t('agentSettings.workbench.searchConfigured')} aria-label={t('agentSettings.workbench.searchConfigured')} onInput={(event) => setSearch(event.currentTarget.value)} /></label>
      <Select size='small' className={styles.modeFilter} value={filter} aria-label={t('agentSettings.workbench.filterStatus')} onChange={setFilter} options={[
        { value: 'all', label: t('agentSettings.workbench.allModes') },
        { value: 'initial', label: t('agentSettings.capabilities.initialShort') },
        { value: 'on_demand', label: t('agentSettings.capabilities.onDemandShort') },
        { value: 'unavailable', label: t('agentSettings.workbench.needsAttention') },
      ]} />
      <Button type='primary' icon={<Add theme='outline' size={16} />} disabled={disabled} onClick={openPicker}>{t('agentSettings.workbench.addCapabilities')}</Button>
    </div>

    <nav className={styles.categoryTabs} aria-label={t('agentSettings.workbench.category')}>
      <button type='button' aria-pressed={category === 'all'} onClick={() => setCategory('all')}>{t('agentSettings.workbench.allCategories')}<span>{selected.length}</span></button>
      {CAPABILITY_CATEGORIES.filter((key) => selected.some((reference) => capabilityCategory(reference) === key)).map((key) => <button type='button' key={key} aria-pressed={category === key} onClick={() => setCategory(key)}>{categoryName(key)}<span>{selected.filter((reference) => capabilityCategory(reference) === key).length}</span></button>)}
    </nav>

    {checkedReferences.length > 0 ? <div className={styles.bulkBar}>
      <strong>{t('agentSettings.workbench.selectedCount', { count: checkedReferences.length })}</strong>
      <Button size='mini' disabled={disabled || anyCheckedUnavailable} onClick={() => setMode(checkedReferences, 'initial')}>{t('agentSettings.capabilities.initialShort')}</Button>
      <Button size='mini' disabled={disabled || anyCheckedUnavailable} onClick={() => setMode(checkedReferences, 'on_demand')}>{t('agentSettings.capabilities.onDemandShort')}</Button>
      <Button size='mini' status='danger' disabled={disabled} onClick={() => setMode(checkedReferences, 'none')}>{t('agentSettings.workbench.removeSelected')}</Button>
      <Button size='mini' type='text' onClick={() => setChecked(new Set())}>{t('agentSettings.workbench.clearSelection')}</Button>
    </div> : pageRows.length > 0 && <div className={styles.listMeta}>
      <Checkbox checked={false} disabled={disabled} onChange={() => setChecked(new Set(pageRows.map(capabilityReferenceKey)))}>{t('agentSettings.workbench.selectPage')}</Checkbox>
      <span>{t('agentSettings.workbench.results', { count: filtered.length })}</span>
    </div>}

    <div className={styles.groups}>
      {CAPABILITY_CATEGORIES.map((key) => {
        const rows = pageRows.filter((reference) => capabilityCategory(reference) === key);
        if (!rows.length) return null;
        return <section key={key} className={styles.group} aria-label={categoryName(key)}>
          <div className={styles.groupHeading}><CapabilityCategoryIcon category={key} size={15} /><h3>{categoryName(key)}</h3><span>{filtered.filter((reference) => capabilityCategory(reference) === key).length}</span></div>
          {rows.map((reference) => {
            const identity = capabilityReferenceKey(reference); const item = byKey.get(identity); const name = nameOf(reference); const available = capabilityIsAvailable(item);
            return <div className={`${styles.capabilityRow} ${checked.has(identity) ? styles.rowChecked : ''}`} key={identity}>
              <Checkbox checked={checked.has(identity)} disabled={disabled} aria-label={t('agentSettings.workbench.selectCapability', { name })} onChange={(value: boolean) => toggleChecked(identity, value)} />
              <div className={styles.rowCopy}>
                <div className={styles.rowTitle}><button type='button' onClick={() => setDetails(reference)}>{name}</button>{!available && <span className={styles.unavailable}>{statusLabelFor(reference, item)}</span>}</div>
                <p>{item ? capabilityProductCopy(item, i18n.language).description : statusReasonFor(reference, item)}</p>
              </div>
              <div className={styles.rowActions}>
                <Select size='small' value={capabilityPlacement(document, reference)} disabled={disabled} aria-label={t('agentSettings.capabilities.modeAria', { name })} onChange={(mode: CapabilityPlacement) => setMode([reference], mode)} options={[
                  { value: 'initial', label: t('agentSettings.capabilities.initialShort'), disabled: !available },
                  { value: 'on_demand', label: t('agentSettings.capabilities.onDemandShort'), disabled: !available },
                  { value: 'none', label: t('agentSettings.workbench.disable') },
                ]} />
                <button className={styles.removeButton} type='button' disabled={disabled} aria-label={t('agentSettings.workbench.removeCapability', { name })} title={t('agentSettings.workbench.removeCapability', { name })} onClick={() => setMode([reference], 'none')}><Delete theme='outline' size={16} /></button>
              </div>
            </div>;
          })}
        </section>;
      })}
      {!pageRows.length && <div className={styles.empty} role='status'><span className={styles.emptyIcon}><Connection theme='outline' size={28} /></span><h3>{t(selected.length ? 'agentSettings.workbench.noResults' : 'agentSettings.workbench.emptyTitle')}</h3><p>{t(selected.length ? 'agentSettings.workbench.noResultsHint' : 'agentSettings.workbench.emptyHint')}</p><Button disabled={disabled} onClick={selected.length ? () => { setSearch(''); setCategory('all'); setFilter('all'); } : openPicker}>{t(selected.length ? 'agentSettings.workbench.clearFilters' : 'agentSettings.workbench.addCapabilities')}</Button></div>}
    </div>
    {filtered.length > PAGE_SIZE && <Pagination className={styles.pagination} current={page} pageSize={PAGE_SIZE} total={filtered.length} onChange={setPage} size='small' />}
    <p className={styles.footnote}>{t('agentSettings.workbench.removalHint')}</p>

    <Drawer title={t('agentSettings.workbench.addCapabilities')} visible={pickerOpen} width='min(880px, calc(100vw - 24px))' className={styles.libraryDrawer} onCancel={() => setPickerOpen(false)} unmountOnExit footer={<div className={styles.drawerFooter}><span>{t('agentSettings.workbench.additionCount', { count: additions.size })}</span><div><Button onClick={() => setPickerOpen(false)}>{t('common.cancel')}</Button><Button type='primary' disabled={disabled || additions.size === 0} onClick={applyAdditions}>{t('agentSettings.workbench.addSelected', { count: additions.size })}</Button></div></div>}>
      <div className={styles.pickerIntro}><h3>{t('agentSettings.workbench.libraryTitle')}</h3><p>{t('agentSettings.workbench.libraryHint')}</p></div>
      <label className={styles.searchField}><Search theme='outline' size={17} /><input type='search' value={pickerSearch} placeholder={t('agentSettings.workbench.searchLibrary')} aria-label={t('agentSettings.workbench.searchLibrary')} onInput={(event) => setPickerSearch(event.currentTarget.value)} /></label>
      <div className={styles.pickerLayout}>
        <nav className={styles.pickerCategories} aria-label={t('agentSettings.workbench.libraryCategories')}>
          <button type='button' aria-pressed={pickerCategory === 'all'} onClick={() => setPickerCategory('all')}><Connection theme='outline' size={17} /><span>{t('agentSettings.workbench.allCategories')}</span><small>{candidates.length}</small></button>
          {CAPABILITY_CATEGORIES.map((key) => <button type='button' key={key} aria-pressed={pickerCategory === key} onClick={() => setPickerCategory(key)}><CapabilityCategoryIcon category={key} size={17} /><span>{categoryName(key)}</span><small>{candidates.filter((item) => capabilityCategory(item.capability) === key).length}</small></button>)}
        </nav>
        <div className={styles.pickerResults}>
          <div className={styles.pickerFilters}><Select size='small' value={pickerSource} aria-label={t('agentSettings.workbench.filterSource')} onChange={setPickerSource} options={[
            { value: 'all', label: t('agentSettings.workbench.allSources') }, { value: 'builtin', label: t('agentSettings.workbench.builtin') }, { value: 'plugin', label: t('agentSettings.workbench.plugin') },
          ]} /><Checkbox checked={showUnavailable} onChange={setShowUnavailable}>{t('agentSettings.workbench.showUnavailable')}</Checkbox></div>
          <div className={styles.listMeta}><span>{t('agentSettings.workbench.results', { count: pickerFiltered.length })}</span><Button type='text' size='mini' onClick={() => setAdditions((current) => new Set([...current, ...pickerRows.filter((item) => capabilityIsAvailable(item) && !selectedKeys.has(capabilityReferenceKey(item.capability))).map((item) => capabilityReferenceKey(item.capability))]))}>{t('agentSettings.workbench.selectPage')}</Button></div>
          {pickerRows.map((item) => {
            const reference = item.capability; const identity = capabilityReferenceKey(reference); const included = selectedKeys.has(identity); const available = capabilityIsAvailable(item); const copy = capabilityProductCopy(item, i18n.language);
            return <div className={`${styles.pickerCard} ${additions.has(identity) ? styles.pickerCardSelected : ''}`} key={identity}>
              <label className={styles.pickerCardMain}><input type='checkbox' checked={included || additions.has(identity)} disabled={disabled || included || !available} aria-label={t('agentSettings.workbench.addCapability', { name: copy.name })} onChange={(event) => setAdditions((previous) => { const next = new Set(previous); if (event.target.checked) next.add(identity); else next.delete(identity); return next; })} /><div><strong>{copy.name}</strong><p>{copy.description}</p><span className={styles.sourceHint}>{t(isBuiltinCapability(item) ? 'agentSettings.workbench.builtin' : 'agentSettings.workbench.plugin')} · {categoryName(capabilityCategory(reference))}</span></div>{included ? <Tag size='small'>{t('agentSettings.workbench.alreadyAdded')}</Tag> : !available && <span className={styles.unavailable}>{statusLabel(item)}</span>}</label>
              <details className={styles.pickerDetail}><summary>{t('agentSettings.workbench.viewDetails')}</summary>{renderDetails(reference)}</details>
            </div>;
          })}
          {!pickerRows.length && <div className={styles.empty}><h3>{t('agentSettings.workbench.noResults')}</h3><p>{t('agentSettings.workbench.noResultsHint')}</p></div>}
          {pickerFiltered.length > PAGE_SIZE && <Pagination className={styles.pagination} current={pickerPage} pageSize={PAGE_SIZE} total={pickerFiltered.length} onChange={setPickerPage} size='small' />}
        </div>
      </div>
    </Drawer>
    <Drawer title={details ? nameOf(details) : ''} visible={details !== null} width='min(440px, calc(100vw - 24px))' onCancel={() => setDetails(null)} unmountOnExit footer={null}>{details && renderDetails(details)}</Drawer>
  </div>;
};

export default AgentCapabilityWorkspace;
