/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { Input } from '@arco-design/web-react';
import { Plus } from '@icon-park/react';
import NomiSelect from '@/renderer/components/base/NomiSelect';
import SegmentedTabs, { type SegmentedTabItem } from '@/renderer/components/base/SegmentedTabs';
import type { ICompanionMemoryKind, ICompanionMemorySort } from '@/common/adapter/ipcBridge';
import { MEMORY_KINDS, MEMORY_STATUS_FILTERS, type MemoryStatusFilter } from './constants';

/** Pill search field — matches the round search input used across the app. */
const SEARCH_PILL_CLASS =
  'w-240px max-w-full [&_.arco-input-inner-wrapper]:!rounded-full [&_.arco-input-inner-wrapper]:!border [&_.arco-input-inner-wrapper]:!border-solid [&_.arco-input-inner-wrapper]:!border-[var(--color-border-2)] [&_.arco-input-inner-wrapper:hover]:!border-[var(--color-border-3)] [&_.arco-input-inner-wrapper-focus]:!border-primary-6';

interface MemoryToolbarProps {
  q: string;
  onQChange: (value: string) => void;
  kind: '' | ICompanionMemoryKind;
  onKindChange: (value: '' | ICompanionMemoryKind) => void;
  status: MemoryStatusFilter;
  onStatusChange: (value: MemoryStatusFilter) => void;
  sort: ICompanionMemorySort;
  onSortChange: (value: ICompanionMemorySort) => void;
  /** Quiet action: opens the duplicate-merge assistant in the detail pane. */
  onOpenMerge: () => void;
  mergeOpen: boolean;
  onAdd: () => void;
}

/**
 * One filter row: status segments · search · kind · sort on the left, the quiet
 * merge action and the single primary CTA on the right. No second row of chrome.
 */
const MemoryToolbar: React.FC<MemoryToolbarProps> = ({
  q,
  onQChange,
  kind,
  onKindChange,
  status,
  onStatusChange,
  sort,
  onSortChange,
  onOpenMerge,
  mergeOpen,
  onAdd,
}) => {
  const { t } = useTranslation();

  const statusItems: SegmentedTabItem[] = useMemo(
    () =>
      MEMORY_STATUS_FILTERS.map((value) => ({
        key: value,
        label:
          value === 'active'
            ? t('nomi.memories.statusActive', { defaultValue: '活跃' })
            : value === 'archived'
              ? t('nomi.memories.statusArchived', { defaultValue: '已归档' })
              : t('nomi.memory.statusAll', { defaultValue: '全部' }),
      })),
    [t]
  );

  return (
    <div className='flex flex-wrap items-center gap-x-10px gap-y-8px'>
      <SegmentedTabs
        size='sm'
        items={statusItems}
        activeKey={status}
        onChange={(key) => onStatusChange(key as MemoryStatusFilter)}
      />

      <Input.Search
        allowClear
        value={q}
        onChange={onQChange}
        placeholder={t('nomi.memories.searchPlaceholder', { defaultValue: '搜索记忆内容' })}
        className={SEARCH_PILL_CLASS}
      />

      <NomiSelect
        contentFit
        contentMaxWidth={150}
        value={kind}
        onChange={(value: string) => onKindChange((value || '') as '' | ICompanionMemoryKind)}
        aria-label={t('nomi.memories.kindAll', { defaultValue: '全部类型' })}
      >
        <NomiSelect.Option value=''>{t('nomi.memories.kindAll', { defaultValue: '全部类型' })}</NomiSelect.Option>
        {MEMORY_KINDS.map((item) => (
          <NomiSelect.Option key={item} value={item}>
            {t(`nomi.kinds.${item}`)}
          </NomiSelect.Option>
        ))}
      </NomiSelect>

      <NomiSelect
        contentFit
        contentMaxWidth={150}
        value={sort}
        onChange={(value: ICompanionMemorySort) => onSortChange(value)}
        aria-label={t('nomi.memory.sortLabel', { defaultValue: '排序' })}
      >
        <NomiSelect.Option value='relevance'>{t('nomi.memories.sortRelevance', { defaultValue: '相关性' })}</NomiSelect.Option>
        <NomiSelect.Option value='time'>{t('nomi.memories.sortTime', { defaultValue: '时间' })}</NomiSelect.Option>
        <NomiSelect.Option value='importance'>{t('nomi.memories.sortImportance', { defaultValue: '重要度' })}</NomiSelect.Option>
      </NomiSelect>

      <div className='ml-auto flex items-center gap-10px'>
        {/* Quiet secondary action — the assistant opens in the detail pane. */}
        <div
          role='button'
          tabIndex={0}
          onClick={onOpenMerge}
          onKeyDown={(event) => {
            if (event.key === 'Enter' || event.key === ' ') {
              event.preventDefault();
              onOpenMerge();
            }
          }}
          className={[
            'inline-flex h-30px cursor-pointer select-none items-center rd-8px px-10px text-13px transition-colors',
            mergeOpen
              ? '!bg-primary-1 !text-primary-6'
              : 'text-t-secondary hover:bg-fill-2 hover:text-t-primary active:bg-fill-3',
          ].join(' ')}
        >
          {t('nomi.memories.merge', { defaultValue: '查重合并' })}
        </div>

        <div
          role='button'
          tabIndex={0}
          onClick={onAdd}
          onKeyDown={(event) => {
            if (event.key === 'Enter' || event.key === ' ') {
              event.preventDefault();
              onAdd();
            }
          }}
          className='inline-flex cursor-pointer select-none items-center gap-6px rd-full bg-[rgba(var(--primary-6),0.12)] px-18px py-9px text-13px font-700 leading-none text-[var(--color-text-1)] shadow-[0_6px_18px_rgba(var(--primary-6),0.14)] transition-colors hover:bg-[rgba(var(--primary-6),0.18)]'
        >
          <Plus theme='outline' size='14' fill='currentColor' strokeWidth={3} />
          {t('nomi.memories.add', { defaultValue: '添加记忆' })}
        </div>
      </div>
    </div>
  );
};

export default MemoryToolbar;
