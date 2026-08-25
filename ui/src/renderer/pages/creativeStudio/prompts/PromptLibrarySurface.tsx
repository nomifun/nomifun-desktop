/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Copy, FileText, FolderOpen, Refresh, Search } from '@icon-park/react';
import classNames from 'classnames';
import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';

import {
  filterPromptLibraryItems,
  promptLibraryItemKey,
  promptLibraryFacets,
  toPromptLibrarySelection,
} from './library';
import styles from './PromptLibrary.module.css';
import type { PromptLibraryItem, PromptLibrarySelection } from './types';

export interface PromptLibrarySurfaceProps {
  variant: 'page' | 'sidebar';
  items: readonly PromptLibraryItem[];
  loading?: boolean;
  refreshing?: boolean;
  error?: Error | null;
  invalidCount?: number;
  title?: string;
  description?: string;
  selectedId?: string | null;
  /** Optional source namespace for controlled selections with colliding IDs. */
  selectedSource?: PromptLibraryItem['source'] | null;
  onRetry?: () => void;
  onSelect?: (item: PromptLibraryItem) => void;
  onCopy?: (selection: PromptLibrarySelection) => void;
}

const StateView: React.FC<{
  kind: 'empty' | 'filtered' | 'error';
  detail?: string;
  onReset?: () => void;
  onRetry?: () => void;
}> = ({ kind, detail, onReset, onRetry }) => {
  const { t } = useTranslation();
  const stateCopy = {
    empty: [
      t('creativeStudio.prompts.emptyTitle', {
        defaultValue: 'No prompts available yet',
      }),
      t('creativeStudio.prompts.emptyDescription', {
        defaultValue: 'NomiFun presets and text assets will appear here.',
      }),
    ],
    filtered: [
      t('creativeStudio.prompts.filteredTitle', {
        defaultValue: 'No matching prompts',
      }),
      t('creativeStudio.prompts.filteredDescription', {
        defaultValue: 'Try fewer tags or a different keyword.',
      }),
    ],
    error: [
      t('creativeStudio.prompts.errorTitle', {
        defaultValue: 'Prompt loading failed',
      }),
      detail ||
        t('creativeStudio.prompts.errorFallback', {
          defaultValue: 'Try again later.',
        }),
    ],
  } as const;
  const [title, description] = stateCopy[kind];
  return (
    <div className={styles.state} data-prompt-library-state={kind} role={kind === 'error' ? 'alert' : 'status'}>
      <div>
        <div className={styles.stateIcon} aria-hidden='true'>
          <FolderOpen theme='outline' size={22} fill='currentColor' />
        </div>
        <h3 className={styles.stateTitle}>{title}</h3>
        <p className={styles.stateDescription}>{description}</p>
        {kind === 'filtered' && onReset ? (
          <button type='button' className={styles.resetButton} onClick={onReset}>
            {t('creativeStudio.prompts.clearFilters', {
              defaultValue: 'Clear filters',
            })}
          </button>
        ) : null}
        {kind === 'error' && onRetry ? (
          <button type='button' className={styles.resetButton} onClick={onRetry}>
            <Refresh theme='outline' size={13} fill='currentColor' />
            {t('creativeStudio.prompts.reload', { defaultValue: 'Reload' })}
          </button>
        ) : null}
      </div>
    </div>
  );
};

const PromptCard: React.FC<{
  item: PromptLibraryItem;
  selected: boolean;
  onSelect: () => void;
  onCopy?: () => void;
}> = ({ item, selected, onSelect, onCopy }) => {
  const { t } = useTranslation();

  return (
    <article
      className={classNames(styles.card, selected && styles.selected)}
      data-prompt-library-item={item.id}
      role='listitem'
    >
      <button
        type='button'
        className={styles.cardBody}
        aria-pressed={selected}
        aria-label={t('creativeStudio.prompts.selectPrompt', {
          defaultValue: 'Select prompt: {{title}}',
          title: item.title,
        })}
        onClick={onSelect}
      >
        <div className={styles.cardHeading}>
          <span className={styles.cardIcon} aria-hidden='true'>
            <FileText theme='outline' size={15} fill='currentColor' />
          </span>
          <h3 className={styles.cardTitle}>{item.title}</h3>
          <span className={styles.category}>
            {item.category ??
              t('creativeStudio.prompts.uncategorized', {
                defaultValue: 'Uncategorized',
              })}
          </span>
        </div>
        {item.description ? <p className={styles.cardDescription}>{item.description}</p> : null}
        <p className={styles.promptPreview}>{item.prompt}</p>
        {item.tags.length > 0 ? (
          <div
            className={styles.tagList}
            aria-label={t('creativeStudio.prompts.tagsLabel', {
              defaultValue: 'Tags',
            })}
          >
            {item.tags.slice(0, 4).map((tag) => (
              <span key={tag} className={styles.tag}>
                {tag}
              </span>
            ))}
            {item.tags.length > 4 ? <span className={styles.tag}>+{item.tags.length - 4}</span> : null}
          </div>
        ) : null}
      </button>
      <footer className={styles.cardFooter}>
        <span>
          {item.knowledgeBaseIds.length > 0
            ? t('creativeStudio.prompts.relatedKnowledgeBases', {
                defaultValue: '{{count}} linked knowledge bases',
                count: item.knowledgeBaseIds.length,
              })
            : t('creativeStudio.prompts.copyForReuse', {
                defaultValue: 'Copy for flexible reuse',
              })}
        </span>
        {onCopy ? (
          <button
            type='button'
            className={styles.copyButton}
            aria-label={t('creativeStudio.prompts.copyPrompt', {
              defaultValue: 'Copy prompt: {{title}}',
              title: item.title,
            })}
            data-prompt-library-action='copy'
            onClick={onCopy}
          >
            <Copy theme='outline' size={13} fill='currentColor' />
            {t('creativeStudio.prompts.copy', { defaultValue: 'Copy' })}
          </button>
        ) : null}
      </footer>
    </article>
  );
};

export const PromptLibrarySurface: React.FC<PromptLibrarySurfaceProps> = ({
  variant,
  items,
  loading = false,
  refreshing = false,
  error = null,
  invalidCount = 0,
  title,
  description,
  selectedId,
  selectedSource,
  onRetry,
  onSelect,
  onCopy,
}) => {
  const { t } = useTranslation();
  const [query, setQuery] = useState('');
  const [category, setCategory] = useState<string | null | undefined>(undefined);
  const [selectedTags, setSelectedTags] = useState<string[]>([]);
  const [localSelectedKey, setLocalSelectedKey] = useState<string | null>(null);
  const facets = useMemo(() => promptLibraryFacets(items), [items]);
  const filteredItems = useMemo(
    () => filterPromptLibraryItems(items, { query, category, tags: selectedTags }),
    [category, items, query, selectedTags]
  );
  const hasFilters = Boolean(query.trim() || category !== undefined || selectedTags.length > 0);
  const resolvedTitle =
    title ??
    (variant === 'page'
      ? t('creativeStudio.prompts.pageTitle', {
          defaultValue: 'Prompt library',
        })
      : t('creativeStudio.prompts.sidebarTitle', {
          defaultValue: 'Prompts',
        }));
  const resolvedDescription =
    description ??
    (variant === 'page'
      ? t('creativeStudio.prompts.pageDescription', {
          defaultValue: 'Search and copy prompts for your current creative work.',
        })
      : t('creativeStudio.prompts.sidebarDescription', {
          defaultValue: 'Quickly copy from existing content',
        }));

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

  const reset = useCallback(() => {
    setQuery('');
    setCategory(undefined);
    setSelectedTags([]);
  }, []);

  const select = useCallback(
    (item: PromptLibraryItem) => {
      if (selectedId === undefined) setLocalSelectedKey(promptLibraryItemKey(item));
      onSelect?.(item);
    },
    [onSelect, selectedId]
  );

  const copy = useCallback(
    (item: PromptLibraryItem) => {
      select(item);
      onCopy?.(toPromptLibrarySelection(item));
    },
    [onCopy, select]
  );

  return (
    <section
      className={classNames(styles.surface, variant === 'page' ? styles.page : styles.sidebar)}
      data-prompt-library={variant}
    >
      <div className={styles.inner}>
        <header className={styles.header}>
          <div>
            <h2 className={styles.title}>{resolvedTitle}</h2>
            <p className={styles.description}>{resolvedDescription}</p>
          </div>
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

        <div className={styles.toolbar}>
          <label className={styles.searchField}>
            <Search theme='outline' size={15} fill='currentColor' aria-hidden='true' />
            <input
              type='search'
              value={query}
              aria-label={t('creativeStudio.prompts.search', {
                defaultValue: 'Search prompts',
              })}
              placeholder={t('creativeStudio.prompts.searchPlaceholder', {
                defaultValue: 'Search titles, content, or tags',
              })}
              onChange={(event) => setQuery(event.target.value)}
            />
          </label>

          <div className={styles.facetRow}>
            <span className={styles.facetLabel}>
              {t('creativeStudio.prompts.category', { defaultValue: 'Category' })}
            </span>
            <div className={styles.chips}>
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
            </div>
          </div>

          {facets.tags.length > 0 ? (
            <div className={styles.facetRow}>
              <span className={styles.facetLabel}>
                {t('creativeStudio.prompts.tags', { defaultValue: 'Tags' })}
              </span>
              <div className={styles.chips}>
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
              </div>
            </div>
          ) : null}
        </div>

        {invalidCount > 0 ? (
          <div className={styles.warning} role='status'>
            {t('creativeStudio.prompts.ignoredRecords', {
              defaultValue:
                '{{count}} records were ignored because they did not match the data contract.',
              count: invalidCount,
            })}
          </div>
        ) : null}
        {error && items.length > 0 ? (
          <div className={styles.warning} role='alert'>
            {t('creativeStudio.prompts.refreshFailedStale', {
              defaultValue: 'Refresh failed; showing the last successfully loaded content.',
            })}
          </div>
        ) : null}

        {loading && items.length === 0 ? (
          <div
            className={styles.skeletonGrid}
            data-prompt-library-state='loading'
            role='status'
            aria-label={t('creativeStudio.prompts.loading', {
              defaultValue: 'Loading prompts',
            })}
          >
            {[0, 1, 2, 3].map((item) => (
              <div key={item} className={styles.skeleton} />
            ))}
          </div>
        ) : error && items.length === 0 ? (
          <StateView kind='error' detail={error.message} onRetry={onRetry} />
        ) : items.length === 0 ? (
          <StateView kind='empty' />
        ) : filteredItems.length === 0 ? (
          <StateView kind='filtered' onReset={reset} />
        ) : (
          <>
            <div className={styles.statusLine} aria-live='polite'>
              <span>
                {hasFilters
                  ? t('creativeStudio.prompts.filteredCount', {
                      defaultValue: '{{filtered}} / {{total}} prompts',
                      filtered: filteredItems.length,
                      total: items.length,
                    })
                  : t('creativeStudio.prompts.totalCount', {
                      defaultValue: '{{count}} prompts',
                      count: items.length,
                    })}
              </span>
              {refreshing ? (
                <span>
                  {t('creativeStudio.prompts.refreshing', {
                    defaultValue: 'Refreshing…',
                  })}
                </span>
              ) : null}
            </div>
            <div className={styles.grid} role='list'>
              {filteredItems.map((item) => (
                <PromptCard
                  key={promptLibraryItemKey(item)}
                  item={item}
                  selected={
                    selectedId === undefined
                      ? promptLibraryItemKey(item) === localSelectedKey
                      : item.id === selectedId &&
                        (selectedSource == null || item.source === selectedSource)
                  }
                  onSelect={() => select(item)}
                  onCopy={onCopy ? () => copy(item) : undefined}
                />
              ))}
            </div>
          </>
        )}
      </div>
    </section>
  );
};

export default PromptLibrarySurface;
