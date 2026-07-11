/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * RequirementFilters — compact icon-and-text filter controls for tag, status,
 * sort, and search. When one or more rows are selected, a stable batch-action
 * bar is rendered below the filters (its own surface — never squeezed into the
 * filter row) carrying a Popconfirm-guarded batch delete.
 *
 * Presentational: all state lives in the parent; this only emits callbacks.
 */
import { Button, Dropdown, Input, Menu, Popconfirm } from '@arco-design/web-react';
import type { RefInputType } from '@arco-design/web-react/es/Input/interface';
import { Check, Filter, Search, SortTwo, Tag } from '@icon-park/react';
import React, { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { ITagSummary, RequirementOrderBy, RequirementStatus } from '@/common/adapter/ipcBridge';
import {
  isRequirementSearchExpanded,
  shouldCollapseRequirementSearch,
} from './requirementFilterToolbarState';

const STATUS_OPTIONS: RequirementStatus[] = [
  'pending',
  'in_progress',
  'needs_review',
  'done',
  'failed',
  'cancelled',
];

const DEFAULT_SORT = '__default__';
const ALL_TAGS = '__all_tags__';
const ALL_STATUSES = '__all_statuses__';
const SORT_ASC = '__sort_asc__';
const SORT_DESC = '__sort_desc__';

interface FilterTriggerProps {
  icon: React.ReactNode;
  label: string;
  value?: string;
  onClick?: React.MouseEventHandler<HTMLButtonElement>;
}

export const FilterTrigger: React.FC<FilterTriggerProps> = ({ icon, label, value, onClick }) => (
  <button
    type='button'
    aria-label={value ? `${label}: ${value}` : label}
    onClick={onClick}
    className='inline-flex h-32px max-w-full cursor-pointer items-center gap-6px rounded-6px border-0 bg-transparent px-8px text-13px text-[var(--color-text-2)] transition-colors hover:bg-[var(--color-fill-2)] hover:text-[var(--color-text-1)] focus-visible:outline-2 focus-visible:outline-[rgb(var(--primary-6))]'
  >
    <span aria-hidden='true' className='inline-flex shrink-0'>
      {icon}
    </span>
    <span className='shrink-0'>{label}</span>
    {value && <span className='max-w-160px truncate font-medium text-[var(--color-text-1)]'>{value}</span>}
  </button>
);

interface RequirementFiltersProps {
  tag?: string;
  status?: RequirementStatus;
  search: string;
  orderBy?: RequirementOrderBy;
  order: 'asc' | 'desc';
  onTagChange: (t?: string) => void;
  onStatusChange: (s?: RequirementStatus) => void;
  onSearchChange: (q: string) => void;
  onOrderByChange: (o?: RequirementOrderBy) => void;
  onOrderChange: (dir: 'asc' | 'desc') => void;
  tagOptions: ITagSummary[];
  selectedCount: number;
  onBatchDelete: () => void; // shown only when selectedCount>0, as a stable bar
}

// Same selected-surface principle as list rows: subtle feedback, readable text.
const SOFT_BATCH_BAR_STYLE: React.CSSProperties = {
  background:
    'linear-gradient(rgba(var(--primary-6), 0.06), rgba(var(--primary-6), 0.06)), var(--color-bg-2)',
  borderColor: 'rgba(var(--primary-6), 0.22)',
};

const RequirementFilters: React.FC<RequirementFiltersProps> = ({
  tag,
  status,
  search,
  orderBy,
  order,
  onTagChange,
  onStatusChange,
  onSearchChange,
  onOrderByChange,
  onOrderChange,
  tagOptions,
  selectedCount,
  onBatchDelete,
}) => {
  const { t } = useTranslation();
  const [searchActive, setSearchActive] = useState(false);
  const searchInputRef = useRef<RefInputType | null>(null);
  const searchExpanded = isRequirementSearchExpanded(searchActive, search);

  useEffect(() => {
    if (searchActive) searchInputRef.current?.focus();
  }, [searchActive]);

  const filterLabel = t('requirements.columns.tag');
  const statusLabel = t('requirements.columns.status');
  const sortLabel = t('requirements.sort.label');
  const searchLabel = t('requirements.searchLabel');
  const selectedStatusLabel = status ? t(`requirements.status.${status}`) : undefined;
  const sortOptions: Array<{ label: string; value: RequirementOrderBy | typeof DEFAULT_SORT }> = [
    { label: t('requirements.sort.default'), value: DEFAULT_SORT },
    { label: t('requirements.sort.byId'), value: 'id' },
    { label: t('requirements.sort.byCreatedAt'), value: 'created_at' },
    { label: t('requirements.sort.byUpdatedAt'), value: 'updated_at' },
    { label: t('requirements.sort.byStatus'), value: 'status' },
  ];
  const selectedSortLabel = orderBy ? sortOptions.find((option) => option.value === orderBy)?.label : undefined;

  const optionContent = (label: React.ReactNode, selected: boolean) => (
    <span className='flex min-w-140px items-center gap-8px'>
      <span className='inline-flex w-14px shrink-0'>{selected && <Check theme='outline' size='14' />}</span>
      <span className='min-w-0 flex-1 truncate'>{label}</span>
    </span>
  );

  const tagMenu = (
    <Menu onClickMenuItem={(key) => onTagChange(key === ALL_TAGS ? undefined : String(key))}>
      <Menu.Item key={ALL_TAGS}>{optionContent(t('requirements.allTags'), !tag)}</Menu.Item>
      {tagOptions.map((item) => (
        <Menu.Item key={item.tag}>
          {optionContent(`${item.tag} (${item.done}/${item.total})`, tag === item.tag)}
        </Menu.Item>
      ))}
    </Menu>
  );

  const statusMenu = (
    <Menu
      onClickMenuItem={(key) =>
        onStatusChange(key === ALL_STATUSES ? undefined : (String(key) as RequirementStatus))
      }
    >
      <Menu.Item key={ALL_STATUSES}>{optionContent(t('requirements.allStatuses'), !status)}</Menu.Item>
      {STATUS_OPTIONS.map((item) => (
        <Menu.Item key={item}>
          {optionContent(t(`requirements.status.${item}`), status === item)}
        </Menu.Item>
      ))}
    </Menu>
  );

  const sortMenu = (
    <Menu
      onClickMenuItem={(key) => {
        if (key === SORT_ASC || key === SORT_DESC) {
          onOrderChange(key === SORT_ASC ? 'asc' : 'desc');
          return;
        }
        onOrderByChange(key === DEFAULT_SORT ? undefined : (String(key) as RequirementOrderBy));
      }}
    >
      <Menu.ItemGroup title={sortLabel}>
        {sortOptions.map((option) => (
          <Menu.Item key={option.value}>
            {optionContent(option.label, (orderBy ?? DEFAULT_SORT) === option.value)}
          </Menu.Item>
        ))}
      </Menu.ItemGroup>
      <Menu.ItemGroup title={t('requirements.sort.direction')}>
        <Menu.Item key={SORT_ASC} disabled={!orderBy}>
          {optionContent(`↑ ${t('requirements.sort.asc')}`, Boolean(orderBy) && order === 'asc')}
        </Menu.Item>
        <Menu.Item key={SORT_DESC} disabled={!orderBy}>
          {optionContent(`↓ ${t('requirements.sort.desc')}`, Boolean(orderBy) && order === 'desc')}
        </Menu.Item>
      </Menu.ItemGroup>
    </Menu>
  );

  return (
    <div className='flex flex-col gap-10px'>
      <div className='flex flex-wrap items-center gap-8px'>
        <Dropdown droplist={tagMenu} trigger='click' position='bl' getPopupContainer={() => document.body}>
          <FilterTrigger
            icon={<Tag theme='outline' size='15' fill='currentColor' />}
            label={filterLabel}
            value={tag}
          />
        </Dropdown>
        <Dropdown droplist={statusMenu} trigger='click' position='bl' getPopupContainer={() => document.body}>
          <FilterTrigger
            icon={<Filter theme='outline' size='15' fill='currentColor' />}
            label={statusLabel}
            value={selectedStatusLabel}
          />
        </Dropdown>
        <Dropdown droplist={sortMenu} trigger='click' position='bl' getPopupContainer={() => document.body}>
          <FilterTrigger
            icon={<SortTwo theme='outline' size='15' fill='currentColor' />}
            label={sortLabel}
            value={selectedSortLabel}
          />
        </Dropdown>
        {searchExpanded ? (
          <Input
            ref={searchInputRef}
            allowClear
            prefix={<Search theme='outline' size='15' fill='currentColor' />}
            placeholder={t('requirements.search')}
            className='max-w-full w-260px'
            value={search}
            onChange={onSearchChange}
            onBlur={() => {
              if (shouldCollapseRequirementSearch(search)) setSearchActive(false);
            }}
            onKeyDown={(event) => {
              if (event.key === 'Escape') {
                setSearchActive(false);
                event.currentTarget.blur();
              }
            }}
          />
        ) : (
          <FilterTrigger
            icon={<Search theme='outline' size='15' fill='currentColor' />}
            label={searchLabel}
            onClick={() => setSearchActive(true)}
          />
        )}
      </div>

      {/* Stable batch-action bar — its own surface, only mounted when there is
          a selection. Kept out of the filter row so the filters never reflow. */}
      {selectedCount > 0 && (
        <div
          className='flex items-center justify-between gap-12px rounded-10px border border-solid border-[var(--color-border-2)] bg-[var(--color-bg-2)] px-12px py-8px'
          style={SOFT_BATCH_BAR_STYLE}
        >
          <span className='text-13px text-[var(--color-text-2)]'>
            {t('requirements.actions.batchDelete', { count: selectedCount })}
          </span>
          <Popconfirm
            title={t('requirements.actions.batchDeleteConfirm', { count: selectedCount })}
            onOk={onBatchDelete}
          >
            <Button status='danger' size='small' shape='round'>
              {t('requirements.actions.delete')}
            </Button>
          </Popconfirm>
        </div>
      )}
    </div>
  );
};

export default RequirementFilters;
