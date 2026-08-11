/**
 * MarketSettingsPanel — shared ranking-market surface for the skill, MCP,
 * plugin, and preset-package markets. Renders an outlined, transparent library
 * surface with the source switcher, sync / search controls, card grid, and
 * (optionally) the shared audience / scenario tag filter bar. Consumers own
 * what "Add" means via `onAdd`.
 */
import { ipcBridge } from '@/common';
import type { ISkillMarketItem, SkillMarketSource } from '@/common/adapter/ipcBridge';
import { resolveLocaleKey } from '@/common/utils';
import { useLayoutContext } from '@/renderer/hooks/context/LayoutContext';
import { usePresetTags } from '@/renderer/hooks/preset';
import { openExternalUrl } from '@/renderer/utils/platform';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import PresetTagFilterBar from './PresetSettings/PresetTagFilterBar';
import type { TagFilterState } from './PresetSettings/presetUtils';
import type { SkillTagFilterState } from './skill/skillFilter';
import SkillMarketCard from './skill/SkillMarketCard';
import {
  ENHANCED_TOOLS_EMPTY_STATE_CLASS,
  ENHANCED_TOOLS_GRID_CLASS,
  ENHANCED_TOOLS_HEADER_CLASS,
  ENHANCED_TOOLS_SURFACE_CLASS,
} from './enhancedToolsLayout';
import {
  cleanMarketText,
  filterSkillMarketItems,
  marketSourceLabel,
  marketSourceUrl,
  normalizeSkillMarketErrors,
  normalizeSkillMarketItems,
  resolveMarketSyncItems,
  selectMarketSourceWithItems,
} from './skill/skillMarket';
import { Button, Input } from '@arco-design/web-react';
import { CloseSmall, LinkOne, Refresh, Search } from '@icon-park/react';
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

const CARD_GRID_COLS = 'repeat(auto-fill, minmax(min(232px, 100%), 1fr))';
const EMPTY_TAG_FILTER: SkillTagFilterState = { audience: [], scenario: [] };

/** Per-market wording overrides; every field falls back to the generic `settings.market.*` copy. */
type MarketPanelTextOverrides = {
  syncSuccess?: string;
  syncKeptCache?: string;
  syncEmpty?: string;
  syncError?: string;
  openFailed?: string;
  openInBrowser?: string;
  /** Shown when a search query matches nothing in the active source. */
  noSearchMatch?: (query: string, sourceLabel: string) => string;
  /** Shown when items exist but the active filters exclude them all. */
  noFilterMatch?: string;
  lastUpdated?: (time: string) => string;
};

type MarketSettingsPanelProps = {
  title: string;
  description: string;
  sources: SkillMarketSource[];
  cacheKey: string;
  autoSyncKey: string;
  defaultSource: SkillMarketSource;
  searchPlaceholder: string;
  emptyText: string;
  onAdd: (item: ISkillMarketItem) => void | Promise<void>;
  /** Render the shared audience/scenario tag filter bar (used by the skill market). */
  enableTagFilter?: boolean;
  /**
   * Stable id fragment for e2e hooks, e.g. `skill-market` keeps the legacy
   * `btn-sync-skill-market` ids. Omit to render without test ids.
   */
  testIdPrefix?: string;
  text?: MarketPanelTextOverrides;
};

const MarketSettingsPanel: React.FC<MarketSettingsPanelProps> = ({
  title,
  description,
  sources,
  cacheKey,
  autoSyncKey,
  defaultSource,
  searchPlaceholder,
  emptyText,
  onAdd,
  enableTagFilter = false,
  testIdPrefix,
  text,
}) => {
  const { t, i18n } = useTranslation();
  const localeKey = resolveLocaleKey(i18n.language);
  const layout = useLayoutContext();
  const isMobile = layout?.isMobile ?? false;
  const tags = usePresetTags();
  const [message, messageContext] = useArcoMessage({ maxCount: 10 });
  const autoSyncStartedRef = useRef(false);
  const itemsRef = useRef<ISkillMarketItem[]>([]);

  const [activeSource, setActiveSource] = useState<SkillMarketSource>(defaultSource);
  const [items, setItems] = useState<ISkillMarketItem[]>([]);
  const [fetchedAt, setFetchedAt] = useState<number | null>(null);
  const [errors, setErrors] = useState<string[]>([]);
  const [loading, setLoading] = useState(false);
  const [searchQuery, setSearchQuery] = useState('');
  const [searchExpanded, setSearchExpanded] = useState(false);
  const [tagFilter, setTagFilter] = useState<TagFilterState>({ audience: [], scenario: [] });
  const pendingAddIdsRef = useRef<Set<string>>(new Set());
  const [pendingAddIds, setPendingAddIds] = useState<Set<string>>(new Set());

  const testId = useCallback(
    (id: string): string | undefined => (testIdPrefix ? id.replace('{market}', testIdPrefix) : undefined),
    [testIdPrefix]
  );

  useEffect(() => {
    try {
      const raw = localStorage.getItem(cacheKey);
      if (!raw) return;
      const cache = JSON.parse(raw) as { fetched_at?: number; items?: unknown; errors?: unknown };
      const cachedItems = normalizeSkillMarketItems(cache.items).filter((item) => sources.includes(item.source));
      itemsRef.current = cachedItems;
      setItems(cachedItems);
      setActiveSource((source) => selectMarketSourceWithItems(source, sources, cachedItems));
      setFetchedAt(typeof cache.fetched_at === 'number' ? cache.fetched_at : null);
      setErrors(normalizeSkillMarketErrors(cache.errors));
    } catch {
      localStorage.removeItem(cacheKey);
    }
  }, [cacheKey, sources]);

  // Drop filter selections whose tags were deleted from the shared vocabulary.
  useEffect(() => {
    if (!enableTagFilter) return;
    const audienceIds = new Set(tags.audienceTags.map((tag) => tag.preset_tag_id));
    const scenarioIds = new Set(tags.scenarioTags.map((tag) => tag.preset_tag_id));
    setTagFilter((prev) => {
      const audience = prev.audience.filter((presetTagId) => audienceIds.has(presetTagId));
      const scenario = prev.scenario.filter((presetTagId) => scenarioIds.has(presetTagId));
      if (audience.length === prev.audience.length && scenario.length === prev.scenario.length) return prev;
      return { audience, scenario };
    });
  }, [enableTagFilter, tags.audienceTags, tags.scenarioTags]);

  const syncMarket = useCallback(
    async (options?: { showToast?: boolean }) => {
      const showToast = options?.showToast ?? true;
      setLoading(true);
      try {
        const result = await ipcBridge.fs.syncSkillMarketRankings.invoke({ sources });
        const normalized = normalizeSkillMarketItems(result.items).filter((item) => sources.includes(item.source));
        const normalizedErrors = normalizeSkillMarketErrors(result.errors);
        const nextItems = resolveMarketSyncItems(itemsRef.current, normalized);
        itemsRef.current = nextItems;
        setItems(nextItems);
        setActiveSource((source) => selectMarketSourceWithItems(source, sources, nextItems));
        setFetchedAt(result.fetched_at);
        setErrors(normalizedErrors);
        localStorage.setItem(
          cacheKey,
          JSON.stringify({
            fetched_at: result.fetched_at,
            items: nextItems,
            errors: normalizedErrors,
          })
        );
        if (showToast) {
          if (normalized.length > 0) {
            message.success(text?.syncSuccess ?? t('settings.market.syncSuccess', { defaultValue: '市场已更新' }));
          } else if (nextItems.length > 0) {
            message.warning(
              text?.syncKeptCache ??
                t('settings.market.syncKeptCache', { defaultValue: '未获取到新数据，已保留本地缓存。' })
            );
          } else {
            message.warning(text?.syncEmpty ?? t('settings.market.syncEmpty', { defaultValue: '未获取到市场数据。' }));
          }
        }
      } catch (error) {
        console.error('Failed to sync market:', error);
        const errorText = text?.syncError ?? t('settings.market.syncError', { defaultValue: '更新市场失败' });
        setErrors([errorText]);
        if (showToast) message.error(errorText);
      } finally {
        setLoading(false);
      }
    },
    [cacheKey, message, sources, t, text]
  );

  useEffect(() => {
    if (autoSyncStartedRef.current) return;
    autoSyncStartedRef.current = true;
    try {
      if (sessionStorage.getItem(autoSyncKey) === '1') return;
      sessionStorage.setItem(autoSyncKey, '1');
    } catch {
      // Storage is an optimization only; the fetch itself is useful.
    }
    void syncMarket({ showToast: false });
  }, [autoSyncKey, syncMarket]);

  const skillTagFilter = useMemo<SkillTagFilterState>(() => {
    if (!enableTagFilter) return EMPTY_TAG_FILTER;
    const keyById = new Map(
      [...tags.audienceTags, ...tags.scenarioTags].map((tag) => [tag.preset_tag_id, tag.key] as const)
    );
    return {
      audience: tagFilter.audience.flatMap((presetTagId) => {
        const key = keyById.get(presetTagId);
        return key ? [key] : [];
      }),
      scenario: tagFilter.scenario.flatMap((presetTagId) => {
        const key = keyById.get(presetTagId);
        return key ? [key] : [];
      }),
    };
  }, [enableTagFilter, tagFilter, tags.audienceTags, tags.scenarioTags]);

  const filteredItems = useMemo(
    () => filterSkillMarketItems(items, activeSource, searchQuery, skillTagFilter),
    [items, activeSource, searchQuery, skillTagFilter]
  );

  const sourceCounts = useMemo(() => {
    const counts: Partial<Record<SkillMarketSource, number>> = {};
    for (const source of sources) counts[source] = 0;
    for (const item of items) {
      if (sources.includes(item.source)) {
        counts[item.source] = (counts[item.source] ?? 0) + 1;
      }
    }
    return counts;
  }, [items, sources]);

  const handleOpenMarket = useCallback(async () => {
    try {
      await openExternalUrl(marketSourceUrl(activeSource));
    } catch (error) {
      console.error('Failed to open market:', error);
      message.error(text?.openFailed ?? t('settings.market.openFailed', { defaultValue: '无法打开市场' }));
    }
  }, [activeSource, message, t, text]);

  const handleMarketAdd = useCallback(
    async (item: ISkillMarketItem) => {
      if (pendingAddIdsRef.current.has(item.id)) return;
      const started = new Set(pendingAddIdsRef.current);
      started.add(item.id);
      pendingAddIdsRef.current = started;
      setPendingAddIds(started);
      try {
        await onAdd(item);
      } catch (error) {
        // Consumers normally own their user-facing error message. Keep the
        // shared callback boundary rejection-safe for future consumers.
        console.error('Market add callback failed:', error);
      } finally {
        const finished = new Set(pendingAddIdsRef.current);
        finished.delete(item.id);
        pendingAddIdsRef.current = finished;
        setPendingAddIds(finished);
      }
    },
    [onAdd]
  );

  const isSearchVisible = searchExpanded || searchQuery.length > 0;
  const activeSearch = searchQuery.trim().length > 0;
  const resolvedEmptyText = loading
    ? t('common.loading', { defaultValue: '加载中...' })
    : activeSearch
      ? (text?.noSearchMatch?.(searchQuery.trim(), marketSourceLabel(activeSource)) ??
        t('settings.market.noSearchMatch', {
          query: searchQuery.trim(),
          source: marketSourceLabel(activeSource),
          defaultValue: `${marketSourceLabel(activeSource)} 中未找到与“${searchQuery.trim()}”相关的条目。`,
        }))
      : items.length === 0
        ? emptyText
        : (text?.noFilterMatch ?? emptyText);

  const marketSourceSwitcher = (
    <div
      data-testid={testId('{market}-source-actions')}
      className='inline-flex flex-none items-center gap-4px rounded-12px border border-solid border-[var(--color-border-2)] bg-[var(--color-bg-2)] p-3px'
    >
      {sources.map((source) => (
        <Button
          key={source}
          size='small'
          type={activeSource === source ? 'primary' : 'text'}
          data-testid={testId(`btn-{market}-source-${source}`)}
          className='!rounded-9px !h-28px !px-12px !text-12px'
          onClick={() => setActiveSource(source)}
        >
          {marketSourceLabel(source)}
          {(sourceCounts[source] ?? 0) > 0 ? ` ${sourceCounts[source]}` : ''}
        </Button>
      ))}
    </div>
  );

  const marketIconActions = (
    <div
      data-testid={testId('{market}-icon-actions')}
      className={`flex items-center gap-10px ${isMobile ? 'w-full flex-wrap justify-end' : 'ml-auto flex-none justify-end'}`}
    >
      <Button
        type={isSearchVisible ? 'secondary' : 'text'}
        size='small'
        data-testid={testId('btn-search-{market}')}
        className='!rounded-10px !h-34px !w-34px !p-0 flex items-center justify-center !text-t-secondary hover:!bg-fill-1 hover:!text-t-primary'
        icon={isSearchVisible ? <CloseSmall size={16} fill='currentColor' /> : <Search size={16} fill='currentColor' />}
        onClick={() => {
          if (isSearchVisible) {
            setSearchExpanded(false);
            setSearchQuery('');
            return;
          }
          setSearchExpanded(true);
        }}
      />
      <Button
        type='text'
        size='small'
        data-testid={testId('btn-sync-{market}')}
        className='!rounded-10px !h-34px !w-34px !p-0 flex items-center justify-center !text-t-secondary hover:!bg-fill-1 hover:!text-t-primary'
        icon={<Refresh size={16} fill='currentColor' className={loading ? 'animate-spin' : ''} />}
        onClick={() => void syncMarket()}
        title={t('common.refresh', { defaultValue: '刷新' })}
      />
    </div>
  );

  const marketActions = (
    <div
      data-testid={testId('{market}-actions')}
      className={`flex items-center gap-10px ${isMobile ? 'w-full flex-wrap' : 'ml-auto flex-none justify-end'}`}
    >
      {marketSourceSwitcher}
      {marketIconActions}
    </div>
  );

  return (
    <div
      aria-label={title}
      data-testid={testId('{market}-surface')}
      className={ENHANCED_TOOLS_SURFACE_CLASS}
    >
      {messageContext}
      <div className={ENHANCED_TOOLS_HEADER_CLASS}>
        <div
          data-testid={testId('{market}-header-row')}
          className={`flex gap-12px ${isMobile ? 'flex-col' : 'items-center justify-between'}`}
        >
          <div className='min-w-0'>
            <p
              data-testid={testId('{market}-description')}
              className='m-0 max-w-[680px] text-14px text-t-secondary leading-relaxed'
            >
              {description}
            </p>
          </div>
          {enableTagFilter ? marketIconActions : marketActions}
        </div>

        {isSearchVisible && (
          <Input
            allowClear
            autoFocus
            value={searchQuery}
            data-testid={testId('input-search-{market}')}
            className='!bg-[var(--color-bg-2)]'
            placeholder={searchPlaceholder}
            prefix={<Search size={14} fill='currentColor' />}
            onChange={(value) => setSearchQuery(cleanMarketText(value, 80))}
          />
        )}

        {enableTagFilter && (
          <PresetTagFilterBar
            audienceTags={tags.audienceTags}
            scenarioTags={tags.scenarioTags}
            value={tagFilter}
            onChange={setTagFilter}
            localeKey={localeKey}
            onManageTags={() => undefined}
            hideManageTags
            actions={marketSourceSwitcher}
          />
        )}
      </div>

      {errors.length > 0 && (
        <div className='mb-14px rounded-12px border border-solid border-[rgba(var(--orange-6),0.24)] bg-[rgba(var(--orange-6),0.08)] px-14px py-10px text-12px leading-18px text-[rgba(var(--orange-7),1)]'>
          {errors.join(' / ')}
        </div>
      )}

      {filteredItems.length > 0 ? (
        <div className={ENHANCED_TOOLS_GRID_CLASS} style={{ gridTemplateColumns: CARD_GRID_COLS }}>
          {filteredItems.map((item) => (
            <SkillMarketCard
              key={item.id}
              item={item}
              tagByKey={tags.tagByKey}
              localeKey={localeKey}
              adding={pendingAddIds.has(item.id)}
              onAdd={(marketItem) => void handleMarketAdd(marketItem)}
            />
          ))}
        </div>
      ) : (
        <div className={ENHANCED_TOOLS_EMPTY_STATE_CLASS}>{resolvedEmptyText}</div>
      )}

      {(fetchedAt || items.length > 0) && (
        <div className='mt-12px flex items-center justify-between gap-10px text-12px text-t-tertiary'>
          <span>
            {fetchedAt
              ? (text?.lastUpdated?.(new Date(fetchedAt).toLocaleString()) ??
                t('settings.market.lastUpdated', {
                  time: new Date(fetchedAt).toLocaleString(),
                  defaultValue: '上次更新：{{time}}',
                }))
              : ''}
          </span>
          <Button
            type='text'
            size='mini'
            data-testid={testId('btn-open-{market}-browser')}
            className='!rounded-10px !px-10px !h-28px !text-12px !text-t-secondary hover:!bg-fill-1 hover:!text-t-primary'
            icon={<LinkOne size={14} fill='currentColor' />}
            onClick={() => void handleOpenMarket()}
          >
            {text?.openInBrowser ?? t('settings.market.openInBrowser', { defaultValue: '打开市场' })}
          </Button>
        </div>
      )}
    </div>
  );
};

export default MarketSettingsPanel;
