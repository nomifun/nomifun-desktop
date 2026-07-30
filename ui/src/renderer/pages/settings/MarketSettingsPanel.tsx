import { ipcBridge } from '@/common';
import type { ISkillMarketItem, SkillMarketSource } from '@/common/adapter/ipcBridge';
import { resolveLocaleKey } from '@/common/utils';
import { useLayoutContext } from '@/renderer/hooks/context/LayoutContext';
import { usePresetTags } from '@/renderer/hooks/preset';
import { openExternalUrl } from '@/renderer/utils/platform';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import SkillMarketCard from './skill/SkillMarketCard';
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

  const syncMarket = useCallback(async (options?: { showToast?: boolean }) => {
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
          message.success(t('settings.market.syncSuccess', { defaultValue: '市场已更新' }));
        } else if (nextItems.length > 0) {
          message.warning(t('settings.market.syncKeptCache', { defaultValue: '未获取到新数据，已保留本地缓存。' }));
        } else {
          message.warning(t('settings.market.syncEmpty', { defaultValue: '未获取到市场数据。' }));
        }
      }
    } catch (error) {
      console.error('Failed to sync market:', error);
      const errorText = t('settings.market.syncError', { defaultValue: '更新市场失败' });
      setErrors([errorText]);
      if (showToast) message.error(errorText);
    } finally {
      setLoading(false);
    }
  }, [cacheKey, message, sources, t]);

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

  const filteredItems = useMemo(
    () => filterSkillMarketItems(items, activeSource, searchQuery, { audience: [], scenario: [] }),
    [items, activeSource, searchQuery]
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
      message.error(t('settings.market.openFailed', { defaultValue: '无法打开市场' }));
    }
  }, [activeSource, message, t]);

  const isSearchVisible = searchExpanded || searchQuery.length > 0;
  const activeSearch = searchQuery.trim().length > 0;
  const resolvedEmptyText = loading
    ? t('common.loading', { defaultValue: '加载中...' })
    : activeSearch
      ? t('settings.market.noSearchMatch', {
          query: searchQuery.trim(),
          source: marketSourceLabel(activeSource),
          defaultValue: `${marketSourceLabel(activeSource)} 中未找到与“${searchQuery.trim()}”相关的条目。`,
        })
      : emptyText;

  return (
    <div className={`bg-fill-2 rounded-24px ${isMobile ? 'p-16px' : 'p-20px'}`}>
      {messageContext}
      <div className='flex flex-col gap-16px mb-20px'>
        <div className={`flex gap-12px ${isMobile ? 'flex-col' : 'items-start justify-between'}`}>
          <div className='min-w-0'>
            <h2 className='m-0 text-28px font-700 leading-[1.1] text-t-primary'>{title}</h2>
            <p className='mt-8px mb-0 max-w-[680px] text-14px text-t-secondary leading-relaxed'>{description}</p>
          </div>
          <div className={`flex items-center gap-10px ${isMobile ? 'w-full flex-wrap' : 'flex-shrink-0'}`}>
            <div className='inline-flex items-center gap-4px rounded-12px bg-[var(--color-bg-2)] p-3px border border-solid border-[var(--color-border-2)]'>
              {sources.map((source) => (
                <Button
                  key={source}
                  size='small'
                  type={activeSource === source ? 'primary' : 'text'}
                  className='!rounded-9px !h-28px !px-12px !text-12px'
                  onClick={() => setActiveSource(source)}
                >
                  {marketSourceLabel(source)}
                  {sourceCounts[source] ? ` ${sourceCounts[source]}` : ''}
                </Button>
              ))}
            </div>
            <Button
              type='text'
              size='small'
              className='!rounded-10px !h-34px !w-34px !p-0 flex items-center justify-center !text-t-secondary hover:!bg-fill-1 hover:!text-t-primary'
              icon={<Refresh size={16} fill='currentColor' className={loading ? 'animate-spin' : ''} />}
              onClick={() => void syncMarket()}
              title={t('common.refresh', { defaultValue: 'Refresh' })}
            />
            <Button
              type={isSearchVisible ? 'secondary' : 'text'}
              size='small'
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
          </div>
        </div>

        {isSearchVisible && (
          <Input
            allowClear
            autoFocus
            value={searchQuery}
            className='!bg-[var(--color-bg-2)]'
            placeholder={searchPlaceholder}
            prefix={<Search size={14} fill='currentColor' />}
            onChange={(value) => setSearchQuery(cleanMarketText(value, 80))}
          />
        )}
      </div>

      {errors.length > 0 && (
        <div className='mb-14px rounded-12px border border-solid border-[rgba(var(--orange-6),0.24)] bg-[rgba(var(--orange-6),0.08)] px-14px py-10px text-12px leading-18px text-[rgb(var(--orange-7))]'>
          {errors.join(' / ')}
        </div>
      )}

      {filteredItems.length > 0 ? (
        <div className='grid gap-12px' style={{ gridTemplateColumns: CARD_GRID_COLS }}>
          {filteredItems.map((item) => (
            <SkillMarketCard
              key={item.id}
              item={item}
              tagByKey={tags.tagByKey}
              localeKey={localeKey}
              onAdd={(marketItem) => void onAdd(marketItem)}
            />
          ))}
        </div>
      ) : (
        <div className='text-center text-t-secondary py-40px'>{resolvedEmptyText}</div>
      )}

      {(fetchedAt || items.length > 0) && (
        <div className='mt-16px flex items-center justify-between gap-12px text-12px text-t-tertiary'>
          <span>
            {fetchedAt
              ? t('settings.market.lastUpdated', {
                  time: new Date(fetchedAt).toLocaleString(),
                  defaultValue: '上次更新：{{time}}',
                })
              : ''}
          </span>
          <Button
            type='text'
            size='mini'
            className='!rounded-10px !px-10px !h-28px !text-12px !text-t-secondary hover:!bg-fill-1 hover:!text-t-primary'
            icon={<LinkOne size={14} fill='currentColor' />}
            onClick={() => void handleOpenMarket()}
          >
            {t('settings.market.openInBrowser', { defaultValue: '打开市场' })}
          </Button>
        </div>
      )}
    </div>
  );
};

export default MarketSettingsPanel;
