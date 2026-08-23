/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { FileText, Inbox, LoadingTwo, Refresh, Search } from '@icon-park/react';
import classNames from 'classnames';
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import {
  filterPromptLibraryItems,
  promptLibraryFacets,
  sortPromptLibraryItemsByUpdatedAt,
  type PromptLibraryItem,
  type PromptLibraryPort,
  usePromptLibrary,
} from '..';
import styles from './StandalonePromptLibraryPage.module.css';

const PROMPT_PAGE_SIZE = 30;

const FacetChips: React.FC<{
  children: React.ReactNode;
  contentKey: string;
}> = ({ children, contentKey }) => {
  const chipsRef = useRef<HTMLDivElement>(null);
  const [expanded, setExpanded] = useState(false);
  const [hasOverflow, setHasOverflow] = useState(false);

  useEffect(() => {
    setExpanded(false);
  }, [contentKey]);

  useEffect(() => {
    const chips = chipsRef.current;
    if (!chips) return undefined;

    const measureOverflow = () => {
      const collapsedHeight = Number.parseFloat(
        window.getComputedStyle(chips).getPropertyValue('--facet-chips-collapsed-height')
      );
      setHasOverflow(chips.scrollHeight > collapsedHeight + 1);
    };

    measureOverflow();
    if (typeof ResizeObserver === 'undefined') {
      window.addEventListener('resize', measureOverflow);
      return () => window.removeEventListener('resize', measureOverflow);
    }

    const observer = new ResizeObserver(measureOverflow);
    observer.observe(chips);
    return () => observer.disconnect();
  }, [contentKey]);

  return (
    <div className={styles.facetContent}>
      <div
        ref={chipsRef}
        className={classNames(styles.chips, !expanded && styles.chipsCollapsed)}
        data-facet-chips-expanded={expanded}
      >
        {children}
      </div>
      {hasOverflow ? (
        <button
          type='button'
          className={styles.facetToggle}
          aria-expanded={expanded}
          onClick={() => setExpanded((current) => !current)}
        >
          {expanded ? '收起' : '查看更多'}
        </button>
      ) : null}
    </div>
  );
};

export interface StandalonePromptLibraryAppearanceProps {
  items: readonly PromptLibraryItem[];
  loading?: boolean;
  refreshing?: boolean;
  error?: Error | null;
  invalidCount?: number;
  selectedId?: string | null;
  title?: string;
  onRetry?: () => void;
  onSelect?: (item: PromptLibraryItem) => void;
}

export interface StandalonePromptLibraryPageProps
  extends Omit<
    StandalonePromptLibraryAppearanceProps,
    'items' | 'loading' | 'refreshing' | 'error' | 'invalidCount' | 'onRetry'
  > {
  port: PromptLibraryPort;
  enabled?: boolean;
}

const PromptCard: React.FC<{
  item: PromptLibraryItem;
  selected: boolean;
  onSelect?: (item: PromptLibraryItem) => void;
}> = ({ item, selected, onSelect }) => (
  <article
    className={classNames(styles.card, selected && styles.cardSelected)}
    data-prompt-library-item={item.id}
  >
    <button
      type='button'
      className={styles.cardButton}
      aria-pressed={selected}
      aria-label={`查看提示词：${item.title}`}
      onClick={() => onSelect?.(item)}
    >
      {item.coverUrl ? (
        <div className={styles.cardMedia}>
          <img
            src={item.coverUrl}
            alt={item.title}
            loading='lazy'
            decoding='async'
            referrerPolicy='no-referrer'
          />
        </div>
      ) : (
        <div className={styles.cardMediaFallback} aria-hidden='true'>
          <FileText theme='outline' size={30} fill='currentColor' />
        </div>
      )}
      <div className={styles.cardBody}>
        <div className={styles.cardHeader}>
          <h2 className={styles.cardTitle}>{item.title}</h2>
          {item.updatedAt ? (
            <time className={styles.cardDate} dateTime={new Date(item.updatedAt).toISOString()}>
              {new Intl.DateTimeFormat('zh-CN', {
                year: 'numeric',
                month: '2-digit',
                day: '2-digit',
              }).format(item.updatedAt)}
            </time>
          ) : null}
        </div>
        {item.description ? <p className={styles.cardDescription}>{item.description}</p> : null}
        <p className={styles.promptPreview}>{item.prompt}</p>
        {item.tags.length > 0 ? (
          <div className={styles.cardTags} aria-label='标签'>
            {item.tags.slice(0, 5).map((tag) => (
              <span key={tag}>{tag}</span>
            ))}
          </div>
        ) : null}
      </div>
    </button>
  </article>
);

export const StandalonePromptLibraryAppearance: React.FC<
  StandalonePromptLibraryAppearanceProps
> = ({
  items,
  loading = false,
  refreshing = false,
  error = null,
  invalidCount = 0,
  selectedId = null,
  title = '提示词中心',
  onRetry,
  onSelect,
}) => {
  const [queryInput, setQueryInput] = useState('');
  const [query, setQuery] = useState('');
  const [category, setCategory] = useState<string | null | undefined>(undefined);
  const [selectedTags, setSelectedTags] = useState<string[]>([]);
  const [visibleCount, setVisibleCount] = useState(PROMPT_PAGE_SIZE);
  const facets = useMemo(() => promptLibraryFacets(items), [items]);
  const filteredItems = useMemo(
    () =>
      sortPromptLibraryItemsByUpdatedAt(
        filterPromptLibraryItems(items, { query, category, tags: selectedTags })
      ),
    [category, items, query, selectedTags]
  );
  const hasFilters = Boolean(query.trim() || category !== undefined || selectedTags.length > 0);
  const visibleItems = filteredItems.slice(0, visibleCount);

  useEffect(() => {
    setCategory((current) => {
      if (current === undefined) return current;
      if (current === null) return facets.hasUncategorized ? current : undefined;
      return facets.categories.includes(current) ? current : undefined;
    });
    setSelectedTags((current) => {
      const next = current.filter((tag) => facets.tags.includes(tag));
      return next.length === current.length ? current : next;
    });
  }, [facets]);

  useEffect(() => {
    setVisibleCount(PROMPT_PAGE_SIZE);
  }, [category, items, query, selectedTags]);

  const clearFilters = useCallback(() => {
    setQueryInput('');
    setQuery('');
    setCategory(undefined);
    setSelectedTags([]);
  }, []);

  const handlePageScroll = useCallback(
    (event: React.UIEvent<HTMLElement>) => {
      const target = event.currentTarget;
      if (
        visibleCount < filteredItems.length &&
        target.scrollTop + target.clientHeight >= target.scrollHeight - 160
      ) {
        setVisibleCount((current) =>
          Math.min(filteredItems.length, current + PROMPT_PAGE_SIZE)
        );
      }
    },
    [filteredItems.length, visibleCount]
  );

  return (
    <section
      className={styles.page}
      data-standalone-prompt-library='true'
      onScroll={handlePageScroll}
    >
      <div className={styles.content}>
        <header className={styles.hero}>
          <h1 className={styles.heroTitle}>{title}</h1>
          <p className={styles.heroSubtitle}>
            共 {hasFilters ? filteredItems.length : items.length}{' '}
            条提示词，按标题、标签与分类快速查找灵感。
          </p>
          {onRetry ? (
            <button
              type='button'
              className={styles.refreshButton}
              aria-label='刷新提示词'
              disabled={refreshing}
              onClick={onRetry}
            >
              <Refresh
                theme='outline'
                size={15}
                fill='currentColor'
                className={refreshing ? styles.spinning : undefined}
              />
            </button>
          ) : null}
        </header>

        {loading && items.length === 0 ? (
          <div
            className={styles.centerState}
            data-prompt-page-state='loading'
            role='status'
            aria-label='正在加载提示词'
          >
            <LoadingTwo
              className={styles.spinning}
              theme='outline'
              size={24}
              fill='currentColor'
              aria-hidden='true'
            />
          </div>
        ) : error && items.length === 0 ? (
          <div className={styles.centerState} data-prompt-page-state='error' role='alert'>
            <p>提示词加载失败</p>
            {onRetry ? (
              <button type='button' className={styles.stateAction} onClick={onRetry}>
                重新加载
              </button>
            ) : null}
          </div>
        ) : (
          <div className={styles.libraryBody}>
            <div className={styles.toolbar} data-prompt-page-toolbar='flat'>
              <label className={styles.searchField}>
                <Search theme='outline' size={15} fill='currentColor' aria-hidden='true' />
                <input
                  type='search'
                  value={queryInput}
                  aria-label='搜索提示词'
                  placeholder='按标题查询，按 Enter 搜索'
                  onChange={(event) => setQueryInput(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === 'Enter') setQuery(queryInput);
                  }}
                />
              </label>

              <div className={styles.facetRow}>
                <span className={styles.facetLabel}>分类</span>
                <FacetChips
                  contentKey={`${facets.categories.join('\u0000')}:${facets.hasUncategorized}`}
                >
                  <button
                    type='button'
                    className={classNames(styles.chip, category === undefined && styles.chipActive)}
                    aria-pressed={category === undefined}
                    onClick={() => setCategory(undefined)}
                  >
                    全部
                  </button>
                  {facets.categories.map((item) => (
                    <button
                      key={item}
                      type='button'
                      className={classNames(styles.chip, category === item && styles.chipActive)}
                      aria-pressed={category === item}
                      onClick={() => setCategory(item)}
                    >
                      {item}
                    </button>
                  ))}
                  {facets.hasUncategorized ? (
                    <button
                      type='button'
                      className={classNames(styles.chip, category === null && styles.chipActive)}
                      aria-pressed={category === null}
                      onClick={() => setCategory(null)}
                    >
                      未分类
                    </button>
                  ) : null}
                </FacetChips>
              </div>

              <div className={styles.facetRow}>
                <span className={styles.facetLabel}>标签</span>
                <FacetChips contentKey={facets.tags.join('\u0000')}>
                  <button
                    type='button'
                    className={classNames(styles.chip, selectedTags.length === 0 && styles.chipActive)}
                    aria-pressed={selectedTags.length === 0}
                    onClick={() => setSelectedTags([])}
                  >
                    全部
                  </button>
                  {facets.tags.map((tag) => {
                    const active = selectedTags.includes(tag);
                    return (
                      <button
                        key={tag}
                        type='button'
                        className={classNames(styles.chip, active && styles.chipActive)}
                        aria-pressed={active}
                        onClick={() =>
                          setSelectedTags((current) =>
                            current.includes(tag)
                              ? current.filter((item) => item !== tag)
                              : [...current, tag]
                          )
                        }
                      >
                        {tag}
                      </button>
                    );
                  })}
                </FacetChips>
              </div>
            </div>

            {invalidCount > 0 ? (
              <p className={styles.notice} role='status'>
                已忽略 {invalidCount} 条不符合数据契约的记录。
              </p>
            ) : null}
            {error ? (
              <p className={styles.notice} role='alert'>
                刷新失败，继续显示上一次加载的内容。
              </p>
            ) : null}

            {items.length === 0 ? (
              <div className={styles.loadedEmptyState} data-prompt-page-state='empty' role='status'>
                <Inbox
                  theme='outline'
                  size={48}
                  fill='currentColor'
                  strokeWidth={2}
                  aria-hidden='true'
                />
                <p>没有找到匹配的提示词</p>
              </div>
            ) : (
              <>
                {filteredItems.length === 0 ? (
                  <div
                    className={styles.filteredState}
                    data-prompt-page-state='filtered'
                    role='status'
                  >
                    <p>没有匹配的提示词</p>
                    <button type='button' className={styles.stateAction} onClick={clearFilters}>
                      清除筛选
                    </button>
                  </div>
                ) : (
                  <div className={styles.grid} role='list'>
                    {visibleItems.map((item) => (
                      <div key={item.id} role='listitem'>
                        <PromptCard
                          item={item}
                          selected={selectedId === item.id}
                          onSelect={onSelect}
                        />
                      </div>
                    ))}
                  </div>
                )}
                {filteredItems.length > 0 ? (
                  <p className={styles.loadStatus} aria-live='polite'>
                    {refreshing
                      ? '正在刷新…'
                      : visibleItems.length < filteredItems.length
                        ? '继续向下滚动加载更多'
                        : '已经到底了'}
                  </p>
                ) : null}
              </>
            )}
          </div>
        )}
      </div>
    </section>
  );
};

export const StandalonePromptLibraryPage: React.FC<StandalonePromptLibraryPageProps> = ({
  port,
  enabled = true,
  ...props
}) => {
  const state = usePromptLibrary(port, { enabled });
  return (
    <StandalonePromptLibraryAppearance
      {...props}
      items={state.items}
      loading={state.loading}
      refreshing={state.refreshing}
      error={state.error}
      invalidCount={state.invalidCount}
      onRetry={() => void state.reload()}
    />
  );
};

export default StandalonePromptLibraryPage;
