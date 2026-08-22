/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { CheckOne, FileText, FolderOpen, Refresh, Search } from '@icon-park/react';
import classNames from 'classnames';
import React, { useCallback, useEffect, useMemo, useState } from 'react';

import {
  filterPromptLibraryItems,
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
  onRetry?: () => void;
  onSelect?: (item: PromptLibraryItem) => void;
  onInsert?: (selection: PromptLibrarySelection) => void;
}

const StateView: React.FC<{
  kind: 'empty' | 'filtered' | 'error';
  detail?: string;
  onReset?: () => void;
  onRetry?: () => void;
}> = ({ kind, detail, onReset, onRetry }) => {
  const copy = {
    empty: ['还没有可用的提示词', '可用的 NomiFun 预设和文本素材会显示在这里。'],
    filtered: ['没有匹配的提示词', '尝试减少标签或更换关键词。'],
    error: ['提示词加载失败', detail || '请稍后重试。'],
  } as const;
  const [title, description] = copy[kind];
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
            清除筛选
          </button>
        ) : null}
        {kind === 'error' && onRetry ? (
          <button type='button' className={styles.resetButton} onClick={onRetry}>
            <Refresh theme='outline' size={13} fill='currentColor' />
            重新加载
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
  onInsert?: () => void;
}> = ({ item, selected, onSelect, onInsert }) => (
  <article
    className={classNames(styles.card, selected && styles.selected)}
    data-prompt-library-item={item.id}
    role='listitem'
  >
    <button
      type='button'
      className={styles.cardBody}
      aria-pressed={selected}
      aria-label={`选择提示词：${item.title}`}
      onClick={onSelect}
    >
      <div className={styles.cardHeading}>
        <span className={styles.cardIcon} aria-hidden='true'>
          <FileText theme='outline' size={15} fill='currentColor' />
        </span>
        <h3 className={styles.cardTitle}>{item.title}</h3>
        <span className={styles.category}>{item.category ?? '未分类'}</span>
      </div>
      {item.description ? <p className={styles.cardDescription}>{item.description}</p> : null}
      <p className={styles.promptPreview}>{item.prompt}</p>
      {item.tags.length > 0 ? (
        <div className={styles.tagList} aria-label='标签'>
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
      <span>{item.knowledgeBaseIds.length > 0 ? `关联 ${item.knowledgeBaseIds.length} 个知识库` : '可直接使用'}</span>
      {onInsert ? (
        <button
          type='button'
          className={styles.insertButton}
          aria-label={`插入提示词：${item.title}`}
          onClick={onInsert}
        >
          <CheckOne theme='outline' size={13} fill='currentColor' />
          插入
        </button>
      ) : null}
    </footer>
  </article>
);

export const PromptLibrarySurface: React.FC<PromptLibrarySurfaceProps> = ({
  variant,
  items,
  loading = false,
  refreshing = false,
  error = null,
  invalidCount = 0,
  title = variant === 'page' ? '提示词库' : '提示词',
  description = variant === 'page' ? '搜索并插入适合当前创作的提示词。' : '从已有内容中快速插入',
  selectedId,
  onRetry,
  onSelect,
  onInsert,
}) => {
  const [query, setQuery] = useState('');
  const [category, setCategory] = useState<string | null | undefined>(undefined);
  const [selectedTags, setSelectedTags] = useState<string[]>([]);
  const [localSelectedId, setLocalSelectedId] = useState<string | null>(null);
  const facets = useMemo(() => promptLibraryFacets(items), [items]);
  const filteredItems = useMemo(
    () => filterPromptLibraryItems(items, { query, category, tags: selectedTags }),
    [category, items, query, selectedTags]
  );
  const effectiveSelectedId = selectedId === undefined ? localSelectedId : selectedId;
  const hasFilters = Boolean(query.trim() || category !== undefined || selectedTags.length > 0);

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
      if (selectedId === undefined) setLocalSelectedId(item.id);
      onSelect?.(item);
    },
    [onSelect, selectedId]
  );

  const insert = useCallback(
    (item: PromptLibraryItem) => {
      select(item);
      onInsert?.(toPromptLibrarySelection(item));
    },
    [onInsert, select]
  );

  return (
    <section
      className={classNames(styles.surface, variant === 'page' ? styles.page : styles.sidebar)}
      data-prompt-library={variant}
    >
      <div className={styles.inner}>
        <header className={styles.header}>
          <div>
            <h2 className={styles.title}>{title}</h2>
            <p className={styles.description}>{description}</p>
          </div>
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

        <div className={styles.toolbar}>
          <label className={styles.searchField}>
            <Search theme='outline' size={15} fill='currentColor' aria-hidden='true' />
            <input
              type='search'
              value={query}
              aria-label='搜索提示词'
              placeholder='搜索标题、内容或标签'
              onChange={(event) => setQuery(event.target.value)}
            />
          </label>

          <div className={styles.facetRow}>
            <span className={styles.facetLabel}>分类</span>
            <div className={styles.chips}>
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
            </div>
          </div>

          {facets.tags.length > 0 ? (
            <div className={styles.facetRow}>
              <span className={styles.facetLabel}>标签</span>
              <div className={styles.chips}>
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
              </div>
            </div>
          ) : null}
        </div>

        {invalidCount > 0 ? (
          <div className={styles.warning} role='status'>
            已忽略 {invalidCount} 条不符合数据契约的记录。
          </div>
        ) : null}
        {error && items.length > 0 ? (
          <div className={styles.warning} role='alert'>
            刷新失败，当前仍显示上一次成功加载的内容。
          </div>
        ) : null}

        {loading && items.length === 0 ? (
          <div className={styles.skeletonGrid} data-prompt-library-state='loading' role='status' aria-label='正在加载提示词'>
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
                {hasFilters ? `${filteredItems.length} / ${items.length}` : `${items.length}`} 条提示词
              </span>
              {refreshing ? <span>正在刷新…</span> : null}
            </div>
            <div className={styles.grid} role='list'>
              {filteredItems.map((item) => (
                <PromptCard
                  key={item.id}
                  item={item}
                  selected={item.id === effectiveSelectedId}
                  onSelect={() => select(item)}
                  onInsert={onInsert ? () => insert(item) : undefined}
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
