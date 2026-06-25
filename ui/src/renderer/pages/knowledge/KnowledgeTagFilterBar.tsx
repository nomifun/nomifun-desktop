/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * KnowledgeTagFilterBar — Two-row chip filter bar for the knowledge list page.
 *
 * Row 1: Kind filter (blank / local / web / feishu) with counts + sort control.
 * Row 2: User tag filter with colored dots, counts, + "Manage Tags" entry.
 *
 * Mirrors AssistantTagFilterBar structure. Theme variables only; `<div onClick>`
 * for clickables (no <button>). Active chip: primary-light-1 bg / primary-6 text.
 */
import type { IKnowledgeBase, IKnowledgeTag } from '@/common/adapter/ipcBridge';
import { SettingTwo } from '@icon-park/react';
import React from 'react';
import { useTranslation } from 'react-i18next';
import type { TFunction } from 'i18next';

// ─── Types ───────────────────────────────────────────────────────────────────

export type KnowledgeKind = IKnowledgeBase['kind'];

export type KnowledgeSort = 'updated' | 'created' | 'name' | 'size';

export interface KnowledgeTagFilterBarProps {
  /** Currently selected kind filter (null/undefined = all). */
  kindFilter: KnowledgeKind | null;
  /** Currently selected tag keys (empty = all). */
  tagFilter: string[];
  onKindChange: (kind: KnowledgeKind | null) => void;
  onTagChange: (tags: string[]) => void;
  /** Count per kind (key = kind value, value = count). */
  kindCounts: Record<string, number>;
  /** Count per tag key. */
  tagCounts: Record<string, number>;
  /** Available user tags. */
  tags: IKnowledgeTag[];
  onManageTags: () => void;
  sort: KnowledgeSort;
  onSortChange: (sort: KnowledgeSort) => void;
}

// ─── Kind definitions (ordered) ──────────────────────────────────────────────

const KIND_ORDER: (KnowledgeKind)[] = ['blank', 'local', 'web', 'feishu'];

// ─── Sort labels ─────────────────────────────────────────────────────────────

function useSortLabel(sort: KnowledgeSort, t: TFunction): string {
  switch (sort) {
    case 'updated':
      return t('knowledge.filter.sortUpdated', { defaultValue: '最近更新' });
    case 'created':
      return t('knowledge.filter.sortCreated', { defaultValue: '创建时间' });
    case 'name':
      return t('knowledge.filter.sortName', { defaultValue: '名称' });
    case 'size':
      return t('knowledge.filter.sortSize', { defaultValue: '大小' });
  }
}

const SORT_OPTIONS: KnowledgeSort[] = ['updated', 'created', 'name', 'size'];

// ─── FilterChip ──────────────────────────────────────────────────────────────

const FilterChip: React.FC<{
  label: string;
  active: boolean;
  onClick: () => void;
  count?: number;
  dot?: string; // CSS color for the colored dot (inline style, user-defined color)
}> = ({ label, active, onClick, count, dot }) => (
  <div
    role='button'
    tabIndex={0}
    aria-pressed={active}
    onClick={onClick}
    onKeyDown={(e) => {
      if (e.key === 'Enter' || e.key === ' ') {
        e.preventDefault();
        onClick();
      }
    }}
    className={[
      'inline-flex items-center gap-6px select-none cursor-pointer rounded-full px-12px py-4px text-12px leading-18px',
      'border border-solid transition-all duration-150 whitespace-nowrap',
      active
        ? '!bg-primary-1 !text-primary-6 border-[var(--color-primary-light-3)] font-medium'
        : 'bg-[var(--color-fill-2)] text-[var(--color-text-2)] border-[var(--color-border-2)] hover:bg-[var(--color-fill-3)] hover:text-[var(--color-text-1)]',
    ].join(' ')}
  >
    {dot && (
      <span
        className='inline-block w-7px h-7px rounded-full flex-shrink-0'
        style={{ backgroundColor: dot }}
        aria-hidden='true'
      />
    )}
    {label}
    {count != null && <span className='text-11px opacity-70'>{count}</span>}
  </div>
);

// ─── SortControl ─────────────────────────────────────────────────────────────

const SortControl: React.FC<{
  sort: KnowledgeSort;
  onSortChange: (s: KnowledgeSort) => void;
}> = ({ sort, onSortChange }) => {
  const { t } = useTranslation();
  const label = useSortLabel(sort, t);

  const [open, setOpen] = React.useState(false);
  const ref = React.useRef<HTMLDivElement>(null);

  // Close on outside click
  React.useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [open]);

  return (
    <div ref={ref} className='relative ml-auto flex-shrink-0'>
      <div
        role='button'
        tabIndex={0}
        onClick={() => setOpen((v) => !v)}
        onKeyDown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            setOpen((v) => !v);
          }
        }}
        className='inline-flex items-center gap-4px text-12px text-[var(--color-text-3)] cursor-pointer select-none hover:text-[var(--color-text-2)] transition-colors'
      >
        <span>{t('knowledge.filter.sortLabel', { defaultValue: '排序' })}：{label}</span>
        <span className='text-10px'>▾</span>
      </div>
      {open && (
        <div className='absolute right-0 top-full mt-4px z-50 min-w-100px rounded-8px border border-solid border-[var(--color-border-2)] bg-[var(--color-bg-2)] py-4px shadow-lg'>
          {SORT_OPTIONS.map((opt) => (
            <div
              key={opt}
              role='menuitem'
              onClick={() => {
                onSortChange(opt);
                setOpen(false);
              }}
              className={[
                'px-12px py-6px text-12px cursor-pointer transition-colors',
                opt === sort
                  ? '!text-primary-6 bg-[var(--color-primary-light-1)]'
                  : 'text-[var(--color-text-2)] hover:bg-[var(--color-fill-2)]',
              ].join(' ')}
            >
              {useSortLabel(opt, t)}
            </div>
          ))}
        </div>
      )}
    </div>
  );
};

// ─── Main Component ──────────────────────────────────────────────────────────

const KnowledgeTagFilterBar: React.FC<KnowledgeTagFilterBarProps> = ({
  kindFilter,
  tagFilter,
  onKindChange,
  onTagChange,
  kindCounts,
  tagCounts,
  tags,
  onManageTags,
  sort,
  onSortChange,
}) => {
  const { t } = useTranslation();

  const totalCount = Object.values(kindCounts).reduce((a, b) => a + b, 0);

  const kindLabel = (kind: KnowledgeKind): string => {
    switch (kind) {
      case 'blank':
        return t('knowledge.filter.kindBlank', { defaultValue: '空白' });
      case 'local':
        return t('knowledge.filter.kindLocal', { defaultValue: '本地' });
      case 'web':
        return t('knowledge.filter.kindWeb', { defaultValue: '网页' });
      case 'feishu':
        return t('knowledge.filter.kindFeishu', { defaultValue: '飞书' });
    }
  };

  const toggleTag = (key: string) => {
    const next = tagFilter.includes(key) ? tagFilter.filter((k) => k !== key) : [...tagFilter, key];
    onTagChange(next);
  };

  return (
    <div className='flex flex-col rounded-14px border border-solid border-[var(--color-border-2)] bg-[var(--color-bg-2)] px-14px py-6px'>
      {/* Row 1: Kind filter + sort */}
      <div className='flex items-center gap-9px flex-wrap py-9px'>
        <div className='flex items-center gap-7px flex-shrink-0'>
          <span className='inline-block w-3px h-12px rounded-[2px] bg-[var(--color-primary-light-3)]' aria-hidden='true' />
          <span className='text-11px font-semibold text-[var(--color-text-3)] whitespace-nowrap tracking-wide'>
            {t('knowledge.filter.kindLabel', { defaultValue: '类型' })}
          </span>
        </div>
        <FilterChip
          label={t('knowledge.filter.all', { defaultValue: '全部' })}
          active={kindFilter === null}
          onClick={() => onKindChange(null)}
          count={totalCount}
        />
        {KIND_ORDER.map((kind) => {
          const count = kindCounts[kind];
          if (count == null || count === 0) return null;
          return (
            <FilterChip
              key={kind}
              label={kindLabel(kind)}
              active={kindFilter === kind}
              onClick={() => onKindChange(kindFilter === kind ? null : kind)}
              count={count}
            />
          );
        })}
        <SortControl sort={sort} onSortChange={onSortChange} />
      </div>

      {/* Separator */}
      <div className='border-t border-solid border-[var(--color-border-2)]' />

      {/* Row 2: Tag filter + manage */}
      <div className='flex items-center gap-9px flex-wrap py-9px'>
        <div className='flex items-center gap-7px flex-shrink-0'>
          <span className='inline-block w-3px h-12px rounded-[2px] bg-[var(--color-primary-light-3)]' aria-hidden='true' />
          <span className='text-11px font-semibold text-[var(--color-text-3)] whitespace-nowrap tracking-wide'>
            {t('knowledge.filter.tagLabel', { defaultValue: '标签' })}
          </span>
        </div>
        <FilterChip
          label={t('knowledge.filter.all', { defaultValue: '全部' })}
          active={tagFilter.length === 0}
          onClick={() => onTagChange([])}
        />
        {tags.map((tag) => (
          <FilterChip
            key={tag.key}
            label={tag.label}
            active={tagFilter.includes(tag.key)}
            onClick={() => toggleTag(tag.key)}
            count={tagCounts[tag.key]}
            dot={tag.color}
          />
        ))}
        {/* Manage Tags */}
        <div
          role='button'
          tabIndex={0}
          onClick={onManageTags}
          onKeyDown={(e) => {
            if (e.key === 'Enter' || e.key === ' ') {
              e.preventDefault();
              onManageTags();
            }
          }}
          className={[
            'inline-flex items-center gap-5px select-none cursor-pointer rounded-full px-12px py-4px flex-shrink-0 ml-auto',
            'text-12px font-medium border border-dashed transition-all duration-150',
            'bg-transparent text-[var(--color-text-3)] border-[var(--color-border-3)]',
            'hover:text-[rgb(var(--primary-6))] hover:border-[var(--color-primary-light-3)] hover:bg-[var(--color-primary-light-1)]',
          ].join(' ')}
        >
          <SettingTwo theme='outline' size={13} strokeWidth={3} />
          {t('knowledge.filter.manageTags', { defaultValue: '管理标签' })}
        </div>
      </div>
    </div>
  );
};

export default KnowledgeTagFilterBar;
