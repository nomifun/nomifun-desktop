import type { PluginProductCategory, PluginProductItem, PluginProductStatus } from './pluginProductModel';
import { Button, Input, Select, Switch } from '@arco-design/web-react';
import { AddOne, Code, List, Plug, Pushpin, Search, Upload } from '@icon-park/react';
import React, { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import styles from './PluginProductSurface.module.css';

type SmartView = 'all' | PluginProductStatus;
type ViewMode = 'grid' | 'list';

interface PluginProductHomeProps {
  items: PluginProductItem[];
  loading: boolean;
  search: string;
  busyMountId?: string | null;
  onSearch: (value: string) => void;
  onCreate: (requirement: string) => void;
  onImport: () => void;
  onOpen: (item: PluginProductItem) => void;
  onToggleEnabled: (item: PluginProductItem, enabled: boolean) => void;
  pinnedIds?: ReadonlySet<string>;
  onPin?: (item: PluginProductItem) => void;
}

const categories: PluginProductCategory[] = [
  'knowledge',
  'automation',
  'development',
  'integration',
  'system',
  'other',
];

const smartViews: SmartView[] = ['all', 'enabled', 'disabled', 'draft', 'attention', 'trashed'];

const PluginProductHome: React.FC<PluginProductHomeProps> = ({
  items,
  loading,
  search,
  busyMountId,
  onSearch,
  onCreate,
  onImport,
  onOpen,
  onToggleEnabled,
  pinnedIds,
  onPin,
}) => {
  const { t } = useTranslation();
  const [requirement, setRequirement] = useState('');
  const [smartView, setSmartView] = useState<SmartView>('all');
  const [category, setCategory] = useState<PluginProductCategory | 'all'>('all');
  const [viewMode, setViewMode] = useState<ViewMode>('grid');
  const [sort, setSort] = useState<'updated' | 'name'>('updated');

  const visible = useMemo(() => {
    const filtered = items.filter((item) => {
      if (smartView === 'all' && item.status === 'trashed') return false;
      if (smartView !== 'all' && item.status !== smartView) return false;
      if (category !== 'all' && item.category !== category) return false;
      return true;
    });
    return [...filtered].sort((left, right) =>
      sort === 'name'
        ? left.displayName.localeCompare(right.displayName)
        : right.updatedAtMs - left.updatedAtMs
    );
  }, [category, items, smartView, sort]);

  const countStatus = (status: SmartView) =>
    status === 'all' ? items.filter((item) => item.status !== 'trashed').length : items.filter((item) => item.status === status).length;
  const countCategory = (value: PluginProductCategory) =>
    items.filter((item) => item.category === value).length;

  const submit = () => {
    const value = requirement.trim();
    if (!value) return;
    onCreate(value);
  };

  return (
    <div className={styles.home}>
      <header className={styles.pageHeader}>
        <div>
          <span className={styles.eyebrow}>{t('pluginWorkbench.product.eyebrow')}</span>
          <h2>{t('pluginWorkbench.product.title')}</h2>
          <p>{t('pluginWorkbench.product.subtitle')}</p>
        </div>
        <div className={styles.headerActions}>
          <Button icon={<Upload size={15} />} onClick={onImport}>
            {t('pluginWorkbench.product.import')}
          </Button>
          <Button type='primary' icon={<AddOne size={15} />} onClick={() => onCreate('')}>
            {t('pluginWorkbench.product.create')}
          </Button>
        </div>
      </header>

      <section className={styles.quickCreate} aria-label={t('pluginWorkbench.product.quickCreate')}>
        <span className={styles.quickCreateIcon}><Code theme='outline' size={18} /></span>
        <Input.TextArea
          value={requirement}
          autoSize={{ minRows: 1, maxRows: 4 }}
          placeholder={t('pluginWorkbench.product.requirementPlaceholder')}
          aria-label={t('pluginWorkbench.product.requirementPlaceholder')}
          onChange={setRequirement}
          onInput={(event) => setRequirement(event.currentTarget.value)}
          onPressEnter={(event: React.KeyboardEvent<HTMLTextAreaElement>) => {
            if (!event.shiftKey && !event.nativeEvent.isComposing) {
              event.preventDefault();
              submit();
            }
          }}
        />
        <Button type='primary' disabled={!requirement.trim()} onClick={submit}>
          {t('pluginWorkbench.product.generate')}
        </Button>
      </section>

      <div className={styles.libraryLayout}>
        <aside className={styles.collectionRail} aria-label={t('pluginWorkbench.product.manage')}>
          <Input
            value={search}
            prefix={<Search size={14} />}
            allowClear
            placeholder={t('pluginWorkbench.actions.search')}
            aria-label={t('pluginWorkbench.actions.search')}
            onChange={onSearch}
          />
          <section className={styles.railSection}>
            <h2>{t('pluginWorkbench.product.smartViews')}</h2>
            {smartViews.map((view) => (
              <button
                key={view}
                type='button'
                aria-pressed={smartView === view}
                onClick={() => setSmartView(view)}
              >
                <span>{t(`pluginWorkbench.product.views.${view}`)}</span>
                <small>{countStatus(view)}</small>
              </button>
            ))}
          </section>
          <section className={styles.railSection}>
            <h2>{t('pluginWorkbench.product.categories.title')}</h2>
            <button
              type='button'
              aria-pressed={category === 'all'}
              onClick={() => setCategory('all')}
            >
              <span>{t('pluginWorkbench.product.categories.all')}</span>
              <small>{items.length}</small>
            </button>
            {categories.map((value) => (
              <button
                key={value}
                type='button'
                aria-pressed={category === value}
                onClick={() => setCategory(value)}
              >
                <span>{t(`pluginWorkbench.product.categories.${value}`)}</span>
                <small>{countCategory(value)}</small>
              </button>
            ))}
          </section>
        </aside>

        <main className={styles.libraryContent}>
          <div className={styles.libraryToolbar}>
            <div>
              <h2>{
                category !== 'all'
                  ? t(`pluginWorkbench.product.categories.${category}`)
                  : t(`pluginWorkbench.product.views.${smartView}`)
              }</h2>
              <span>{t('pluginWorkbench.product.resultCount', { count: visible.length })}</span>
            </div>
            <div className={styles.libraryControls}>
              <Select
                size='small'
                value={sort}
                aria-label={t('pluginWorkbench.product.sort.label')}
                onChange={setSort}
                options={[
                  { value: 'updated', label: t('pluginWorkbench.product.sort.updated') },
                  { value: 'name', label: t('pluginWorkbench.product.sort.name') },
                ]}
              />
              <div className={styles.viewSwitch} role='group' aria-label={t('pluginWorkbench.product.viewMode')}>
                <button type='button' aria-label={t('pluginWorkbench.product.gridView')} aria-pressed={viewMode === 'grid'} onClick={() => setViewMode('grid')}>
                  <Plug theme='outline' size={15} />
                </button>
                <button type='button' aria-label={t('pluginWorkbench.product.listView')} aria-pressed={viewMode === 'list'} onClick={() => setViewMode('list')}>
                  <List theme='outline' size={15} />
                </button>
              </div>
            </div>
          </div>

          {visible.length ? (
            <div className={`${styles.pluginCollection} ${viewMode === 'list' ? styles.pluginCollectionList : ''}`}>
              {visible.map((item) => {
                const enabled = item.runtime ? item.runtime.lifecycle === 'enabled' : item.mount ? item.mount.lifecycle === 'enabled' : item.status === 'enabled';
                const installed = item.status !== 'trashed' && item.runtime?.lifecycle !== 'deleting' && Boolean(item.mount?.current || item.runtime?.releases.active);
                const pinned = pinnedIds?.has(item.runtime?.plugin_id ?? item.mount?.mount_id ?? '') ?? false;
                return (
                  <article key={item.key} className={styles.pluginCard}>
                    <div className={styles.pluginCardHeader}>
                      <button type='button' className={styles.pluginIdentity} onClick={() => onOpen(item)}>
                        <span className={styles.pluginIcon}><Plug theme='outline' size={19} /></span>
                        <span>
                          <strong>{item.displayName}</strong>
                          <small>{t(`pluginWorkbench.product.categories.${item.category}`)}</small>
                        </span>
                      </button>
                      {installed ? (
                        <Switch
                          size='small'
                          checked={enabled}
                          loading={busyMountId === item.key}
                          aria-label={t('pluginWorkbench.product.toggle', { name: item.displayName })}
                          onChange={(next: boolean) => onToggleEnabled(item, next)}
                        />
                      ) : (
                        <span className={`${styles.productStatus} ${item.status === 'draft' ? styles.productStatusDraft : ''}`}>
                          {t(`pluginWorkbench.product.views.${item.status}`)}
                        </span>
                      )}
                    </div>
                    <button type='button' className={styles.pluginDescription} onClick={() => onOpen(item)}>
                      {item.description || t('pluginWorkbench.product.noDescription')}
                    </button>
                    <div className={styles.pluginMeta}>
                      {installed && onPin && <button type='button' title={t(pinned ? 'pluginWorkbench.product.unpin' : 'pluginWorkbench.product.pin', { name: item.displayName })} aria-label={t(pinned ? 'pluginWorkbench.product.unpin' : 'pluginWorkbench.product.pin', { name: item.displayName })} aria-pressed={pinned} onClick={() => onPin(item)}><Pushpin theme={pinned ? 'filled' : 'outline'} size={14} /></button>}
                      <span>{
                        item.status === 'attention'
                          ? t('pluginWorkbench.product.views.attention')
                        : item.project?.ready_candidate
                          ? t('pluginWorkbench.product.readyToReview')
                          : item.runtime?.surface_available
                            ? t('pluginWorkbench.product.openReady')
                          : installed
                            ? t('pluginWorkbench.product.capabilityCount', { count: item.contributionCount })
                            : t('pluginWorkbench.product.continueCreating')
                      }</span>
                      <button type='button' onClick={() => onOpen(item)}>
                        {item.status === 'attention' ? t('pluginWorkbench.product.reviewIssue')
                          : item.runtime?.surface_available
                          ? t('pluginWorkbench.product.open')
                          : (item.project || item.runtimeDraft) && !installed
                          ? t('pluginWorkbench.product.continue')
                          : t('pluginWorkbench.product.details')}
                      </button>
                    </div>
                  </article>
                );
              })}
            </div>
          ) : (
            <div className={styles.productEmpty} role='status'>
              <span><Plug theme='outline' size={28} /></span>
              <h2>{loading ? t('pluginWorkbench.states.loadingLibrary') : t('pluginWorkbench.product.emptyTitle')}</h2>
              <p>{t('pluginWorkbench.product.emptyBody')}</p>
              <Button type='primary' icon={<AddOne size={15} />} onClick={() => onCreate('')}>
                {t('pluginWorkbench.product.create')}
              </Button>
            </div>
          )}
        </main>
      </div>
    </div>
  );
};

export default PluginProductHome;
