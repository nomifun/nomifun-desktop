/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * KnowledgeTagFilterBar — Compact toolbar for the knowledge list page.
 *
 * The primary row keeps kind, tag and sort controls together while leaving a
 * responsive action slot for search / management / creation. Selected tags are
 * echoed in a dedicated second row only when the tag filter is active.
 */
import type { IKnowledgeBase, IKnowledgeTag } from '@/common/adapter/ipcBridge';
import { Dropdown, Menu } from '@arco-design/web-react';
import { Check, CloseSmall, Down } from '@icon-park/react';
import type { TFunction } from 'i18next';
import React from 'react';
import { useTranslation } from 'react-i18next';

export type KnowledgeKind = IKnowledgeBase['kind'];

export type KnowledgeSort = 'updated' | 'created' | 'name' | 'size';

export interface KnowledgeTagFilterBarProps {
  kindFilter: KnowledgeKind | null;
  tagFilter: string[];
  onKindChange: (kind: KnowledgeKind | null) => void;
  onTagChange: (tags: string[]) => void;
  kindCounts: Record<string, number>;
  tagCounts: Record<string, number>;
  tags: IKnowledgeTag[];
  sort: KnowledgeSort;
  onSortChange: (sort: KnowledgeSort) => void;
  actions?: React.ReactNode;
}

const KIND_ORDER: KnowledgeKind[] = ['blank', 'local', 'web'];
const SORT_OPTIONS: KnowledgeSort[] = ['updated', 'created', 'name', 'size'];

function getSortLabel(sort: KnowledgeSort, t: TFunction): string {
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

const ToolbarSelect: React.FC<{
  label: string;
  value: string;
  menu: React.ReactNode;
  minWidthClass?: string;
}> = ({ label, value, menu, minWidthClass = 'min-w-148px' }) => (
  <Dropdown trigger='click' position='bl' droplist={menu}>
    <div
      role='button'
      tabIndex={0}
      aria-label={`${label}：${value}`}
      onKeyDown={(event) => {
        if (event.key === 'Enter' || event.key === ' ') {
          event.preventDefault();
          event.currentTarget.click();
        }
      }}
      className={[
        'inline-flex h-38px box-border items-center justify-between gap-12px rounded-10px px-12px',
        'border border-solid border-[var(--color-border-3)] bg-[var(--color-bg-2)]',
        'text-13px text-[var(--color-text-1)] cursor-pointer select-none',
        'hover:border-[var(--color-border-4)] hover:bg-[var(--color-fill-2)]',
        'focus-visible:outline-none focus-visible:border-primary-6 transition-colors',
        minWidthClass,
      ].join(' ')}
    >
      <span className='min-w-0 truncate'>
        <span className='text-[var(--color-text-2)]'>{label}：</span>
        <span className='font-medium'>{value}</span>
      </span>
      <Down theme='outline' size={12} className='flex-none text-[var(--color-text-3)]' />
    </div>
  </Dropdown>
);

const DropdownMenuSurface: React.FC<{ children: React.ReactNode }> = ({ children }) => (
  <div
    className='overflow-hidden rounded-10px border border-solid border-[var(--color-border-2)] bg-[var(--color-bg-2)] shadow-lg'
    style={{ boxShadow: '0 10px 28px rgba(0, 0, 0, 0.14)' }}
  >
    {children}
  </div>
);

const KnowledgeTagFilterBar: React.FC<KnowledgeTagFilterBarProps> = ({
  kindFilter,
  tagFilter,
  onKindChange,
  onTagChange,
  kindCounts,
  tagCounts,
  tags,
  sort,
  onSortChange,
  actions,
}) => {
  const { t } = useTranslation();

  const totalCount = Object.values(kindCounts).reduce((sum, count) => sum + count, 0);
  const allLabel = t('knowledge.filter.all', { defaultValue: '全部' });

  const kindLabel = (kind: KnowledgeKind): string => {
    switch (kind) {
      case 'blank':
        return t('knowledge.filter.kindBlank', { defaultValue: '空白' });
      case 'local':
        return t('knowledge.filter.kindLocal', { defaultValue: '本地' });
      case 'web':
        return t('knowledge.filter.kindWeb', { defaultValue: '网页' });
    }
  };

  const selectedTags = tags.filter((tag) => tagFilter.includes(tag.key));
  const selectedTagSummary = tagFilter.length === 0
    ? allLabel
    : t('knowledge.filter.selectedCount', {
        defaultValue: '已选 {{count}} 个',
        count: tagFilter.length,
      });

  const toggleTag = (key: string) => {
    const next = tagFilter.includes(key) ? tagFilter.filter((tagKey) => tagKey !== key) : [...tagFilter, key];
    onTagChange(next);
  };

  const kindMenu = (
    <DropdownMenuSurface>
      <Menu
        selectedKeys={[kindFilter ?? 'all']}
        onClickMenuItem={(key) => onKindChange(key === 'all' ? null : (String(key) as KnowledgeKind))}
        className='min-w-168px py-4px'
      >
        <Menu.Item key='all'>
          <div className='flex items-center justify-between gap-20px'>
            <span>{allLabel}</span>
            <span className='text-11px text-[var(--color-text-3)]'>{totalCount}</span>
          </div>
        </Menu.Item>
        {KIND_ORDER.map((kind) => (
          <Menu.Item key={kind}>
            <div className='flex items-center justify-between gap-20px'>
              <span>{kindLabel(kind)}</span>
              <span className='text-11px text-[var(--color-text-3)]'>{kindCounts[kind] ?? 0}</span>
            </div>
          </Menu.Item>
        ))}
      </Menu>
    </DropdownMenuSurface>
  );

  const tagMenu = (
    <DropdownMenuSurface>
      <Menu onClickMenuItem={(key) => (key === 'all' ? onTagChange([]) : toggleTag(String(key)))} className='min-w-200px max-h-280px overflow-y-auto py-4px'>
        <Menu.Item key='all'>
          <div className='flex items-center justify-between gap-20px'>
            <span>{allLabel}</span>
            {tagFilter.length === 0 && <Check theme='outline' size={14} className='text-primary-6' />}
          </div>
        </Menu.Item>
        {tags.map((tag) => {
          const active = tagFilter.includes(tag.key);
          return (
            <Menu.Item key={tag.key}>
              <div className='flex items-center justify-between gap-20px'>
                <span className='flex min-w-0 items-center gap-8px'>
                  <span className='h-7px w-7px flex-none rounded-full' style={{ backgroundColor: tag.color }} aria-hidden='true' />
                  <span className='truncate'>{tag.label}</span>
                  <span className='text-11px text-[var(--color-text-3)]'>{tagCounts[tag.key] ?? 0}</span>
                </span>
                {active && <Check theme='outline' size={14} className='flex-none text-primary-6' />}
              </div>
            </Menu.Item>
          );
        })}
      </Menu>
    </DropdownMenuSurface>
  );

  const sortMenu = (
    <DropdownMenuSurface>
      <Menu
        selectedKeys={[sort]}
        onClickMenuItem={(key) => onSortChange(String(key) as KnowledgeSort)}
        className='min-w-168px py-4px'
      >
        {SORT_OPTIONS.map((option) => (
          <Menu.Item key={option}>{getSortLabel(option, t)}</Menu.Item>
        ))}
      </Menu>
    </DropdownMenuSurface>
  );

  return (
    <div className='flex w-full flex-col gap-10px'>
      <div className='flex w-full flex-wrap items-center justify-between gap-10px'>
        <div className='flex flex-wrap items-center gap-8px'>
          <ToolbarSelect
            label={t('knowledge.filter.kindLabel', { defaultValue: '类型' })}
            value={kindFilter ? kindLabel(kindFilter) : allLabel}
            menu={kindMenu}
          />
          <ToolbarSelect
            label={t('knowledge.filter.tagLabel', { defaultValue: '标签' })}
            value={selectedTagSummary}
            menu={tagMenu}
            minWidthClass='min-w-176px'
          />
          <ToolbarSelect
            label={t('knowledge.filter.sortLabel', { defaultValue: '排序' })}
            value={getSortLabel(sort, t)}
            menu={sortMenu}
            minWidthClass='min-w-164px'
          />
        </div>
        {actions}
      </div>

      {tagFilter.length > 0 && (
        <div className='flex min-h-44px w-full box-border flex-wrap items-center gap-8px rounded-14px border border-solid border-[var(--color-border-2)] bg-[var(--color-bg-2)] px-14px py-7px'>
          {selectedTags.map((tag) => (
            <div
              key={tag.key}
              role='button'
              tabIndex={0}
              onClick={() => toggleTag(tag.key)}
              onKeyDown={(event) => {
                if (event.key === 'Enter' || event.key === ' ') {
                  event.preventDefault();
                  toggleTag(tag.key);
                }
              }}
              className='inline-flex items-center gap-7px rounded-full bg-[var(--color-fill-2)] px-11px py-4px text-12px text-[var(--color-text-2)] cursor-pointer hover:bg-[var(--color-fill-3)] hover:text-[var(--color-text-1)] transition-colors'
            >
              <span className='h-7px w-7px flex-none rounded-full' style={{ backgroundColor: tag.color }} aria-hidden='true' />
              <span>{tag.label}</span>
              <CloseSmall theme='outline' size={12} className='text-[var(--color-text-3)]' />
            </div>
          ))}
        </div>
      )}
    </div>
  );
};

export default KnowledgeTagFilterBar;
