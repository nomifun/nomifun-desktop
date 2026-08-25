/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { FileText, Inbox, LoadingTwo, Refresh, Search } from '@icon-park/react';
import classNames from 'classnames';
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import {
  filterPromptLibraryItems,
  promptLibraryItemKey,
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
  const { t } = useTranslation();
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
          {expanded
            ? t('creativeStudio.prompts.collapse', { defaultValue: 'Collapse' })
            : t('creativeStudio.prompts.showMore', { defaultValue: 'Show more' })}
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
  selectedSource?: PromptLibraryItem['source'] | null;
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
}> = ({ item, selected, onSelect }) => {
  const { i18n, t } = useTranslation();
  const locale = i18n.resolvedLanguage ?? i18n.language ?? 'en-US';

  return (
    <article
      className={classNames(styles.card, selected && styles.cardSelected)}
      data-prompt-library-item={item.id}
    >
      <button
        type='button'
        className={styles.cardButton}
        aria-pressed={selected}
        aria-label={t('creativeStudio.prompts.viewPrompt', {
          defaultValue: 'View prompt: {{title}}',
          title: item.title,
        })}
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
                {new Intl.DateTimeFormat(locale, {
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
            <div
              className={styles.cardTags}
              aria-label={t('creativeStudio.prompts.tagsLabel', {
                defaultValue: 'Tags',
              })}
            >
              {item.tags.slice(0, 5).map((tag) => (
                <span key={tag}>{tag}</span>
              ))}
            </div>
          ) : null}
        </div>
      </button>
    </article>
  );
};

export const StandalonePromptLibraryAppearance: React.FC<
  StandalonePromptLibraryAppearanceProps
> = ({
  items,
  loading = false,
  refreshing = false,
  error = null,
  invalidCount = 0,
  selectedId = null,
  selectedSource = null,
  title,
  onRetry,
  onSelect,
}) => {
  const { t } = useTranslation();
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
  const resolvedTitle =
    title ??
    t('creativeStudio.prompts.standaloneTitle', {
      defaultValue: 'Prompt center',
    });

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
          <h1 className={styles.heroTitle}>{resolvedTitle}</h1>
          <p className={styles.heroSubtitle}>
            {t('creativeStudio.prompts.countDescription', {
              defaultValue:
                '{{count}} prompts. Search by title, tags, and category for inspiration.',
              count: hasFilters ? filteredItems.length : items.length,
            })}
          </p>
          {onRetry ? (
            <button
              type='button'
              className={styles.refreshButton}
              aria-label={t('creativeStudio.prompts.refresh', {
                defaultValue: 'Refresh prompts',
              })}
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
            aria-label={t('creativeStudio.prompts.loading', {
              defaultValue: 'Loading prompts',
            })}
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
            <p>
              {t('creativeStudio.prompts.errorTitle', {
                defaultValue: 'Prompt loading failed',
              })}
            </p>
            {onRetry ? (
              <button type='button' className={styles.stateAction} onClick={onRetry}>
                {t('creativeStudio.prompts.reload', { defaultValue: 'Reload' })}
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
                  aria-label={t('creativeStudio.prompts.search', {
                    defaultValue: 'Search prompts',
                  })}
                  placeholder={t('creativeStudio.prompts.standaloneSearchPlaceholder', {
                    defaultValue: 'Search by title, then press Enter',
                  })}
                  onChange={(event) => setQueryInput(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === 'Enter') setQuery(queryInput);
                  }}
                />
              </label>

              <div className={styles.facetRow}>
                <span className={styles.facetLabel}>
                  {t('creativeStudio.prompts.category', { defaultValue: 'Category' })}
                </span>
                <FacetChips
                  contentKey={`${facets.categories.join('\u0000')}:${facets.hasUncategorized}`}
                >
                  <button
                    type='button'
                    className={classNames(styles.chip, category === undefined && styles.chipActive)}
                    aria-pressed={category === undefined}
                    onClick={() => setCategory(undefined)}
                  >
                    {t('creativeStudio.prompts.all', { defaultValue: 'All' })}
                  </button>
                  {facets.categories.map((item) => (
                    <button
                      key={item}
                      type='button'
                      title={item}
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
                      {t('creativeStudio.prompts.uncategorized', {
                        defaultValue: 'Uncategorized',
                      })}
                    </button>
                  ) : null}
                </FacetChips>
              </div>

              <div className={styles.facetRow}>
                <span className={styles.facetLabel}>
                  {t('creativeStudio.prompts.tags', { defaultValue: 'Tags' })}
                </span>
                <FacetChips contentKey={facets.tags.join('\u0000')}>
                  <button
                    type='button'
                    className={classNames(styles.chip, selectedTags.length === 0 && styles.chipActive)}
                    aria-pressed={selectedTags.length === 0}
                    onClick={() => setSelectedTags([])}
                  >
                    {t('creativeStudio.prompts.all', { defaultValue: 'All' })}
                  </button>
                  {facets.tags.map((tag) => {
                    const active = selectedTags.includes(tag);
                    return (
                      <button
                        key={tag}
                        type='button'
                        title={tag}
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
                {t('creativeStudio.prompts.ignoredRecords', {
                  defaultValue:
                    '{{count}} records were ignored because they did not match the data contract.',
                  count: invalidCount,
                })}
              </p>
            ) : null}
            {error ? (
              <p className={styles.notice} role='alert'>
                {t('creativeStudio.prompts.refreshFailedStale', {
                  defaultValue: 'Refresh failed; showing the last loaded content.',
                })}
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
                <p>
                  {t('creativeStudio.prompts.noItems', {
                    defaultValue: 'No matching prompts found',
                  })}
                </p>
              </div>
            ) : (
              <>
                {filteredItems.length === 0 ? (
                  <div
                    className={styles.filteredState}
                    data-prompt-page-state='filtered'
                    role='status'
                  >
                    <p>
                      {t('creativeStudio.prompts.filteredTitle', {
                        defaultValue: 'No matching prompts',
                      })}
                    </p>
                    <button type='button' className={styles.stateAction} onClick={clearFilters}>
                      {t('creativeStudio.prompts.clearFilters', {
                        defaultValue: 'Clear filters',
                      })}
                    </button>
                  </div>
                ) : (
                  <div className={styles.grid} role='list'>
                    {visibleItems.map((item) => (
                      <div key={promptLibraryItemKey(item)} role='listitem'>
                        <PromptCard
                          item={item}
                          selected={
                            selectedId === item.id &&
                            (selectedSource === null || selectedSource === item.source)
                          }
                          onSelect={onSelect}
                        />
                      </div>
                    ))}
                  </div>
                )}
                {filteredItems.length > 0 ? (
                  <p className={styles.loadStatus} aria-live='polite'>
                    {refreshing
                      ? t('creativeStudio.prompts.refreshing', {
                          defaultValue: 'Refreshing…',
                        })
                      : visibleItems.length < filteredItems.length
                        ? t('creativeStudio.prompts.loadMore', {
                            defaultValue: 'Continue scrolling to load more',
                          })
                        : t('creativeStudio.prompts.endOfList', {
                            defaultValue: 'You have reached the end',
                          })}
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
