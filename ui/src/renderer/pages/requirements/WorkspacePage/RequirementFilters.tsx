/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * RequirementFilters — the filter row for the requirements workspace list:
 * a tag Select (labelled `{tag} (done/total)` like the legacy page), a status
 * Select (6 statuses, allowClear), and an `Input.Search`. When one or more
 * rows are selected, a *stable* batch-action bar is rendered below the filters
 * (its own surface — never squeezed into the filter row) carrying a
 * Popconfirm-guarded batch delete.
 *
 * Presentational: all state lives in the parent; this only emits callbacks.
 */
import { Button, Input, Popconfirm, Select } from '@arco-design/web-react';
import React from 'react';
import { useTranslation } from 'react-i18next';

import type { ITagSummary, RequirementStatus } from '@/common/adapter/ipcBridge';

const STATUS_OPTIONS: RequirementStatus[] = [
  'pending',
  'in_progress',
  'needs_review',
  'done',
  'failed',
  'cancelled',
];

interface RequirementFiltersProps {
  tag?: string;
  status?: RequirementStatus;
  search: string;
  onTagChange: (t?: string) => void;
  onStatusChange: (s?: RequirementStatus) => void;
  onSearchChange: (q: string) => void;
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
  onTagChange,
  onStatusChange,
  onSearchChange,
  tagOptions,
  selectedCount,
  onBatchDelete,
}) => {
  const { t } = useTranslation();

  return (
    <div className='flex flex-col gap-10px'>
      <div className='flex flex-wrap items-center gap-8px'>
        <Select
          allowClear
          placeholder={t('requirements.allTags')}
          className='max-w-full'
          style={{ width: 200 }}
          value={tag}
          onChange={(v) => onTagChange(v || undefined)}
          options={tagOptions.map((tg) => ({
            label: `${tg.tag} (${tg.done}/${tg.total})`,
            value: tg.tag,
          }))}
        />
        <Select
          allowClear
          placeholder={t('requirements.allStatuses')}
          className='max-w-full'
          style={{ width: 160 }}
          value={status}
          onChange={(v) => onStatusChange((v as RequirementStatus) || undefined)}
          options={STATUS_OPTIONS.map((s) => ({
            label: t(`requirements.status.${s}`),
            value: s,
          }))}
        />
        <Input.Search
          allowClear
          placeholder={t('requirements.search')}
          className='max-w-full'
          style={{ width: 260 }}
          value={search}
          onChange={(v) => onSearchChange(v)}
          onSearch={(v) => onSearchChange(v)}
        />
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
