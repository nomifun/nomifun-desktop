/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * RequirementListRow — a single refined row in the requirements workspace list
 * (replaces the old dense Arco Table row). Layout, left→right:
 *   checkbox · `#id` badge (tabular-nums) · title + 1-2 line content snippet
 *   · tag chip (mirrors AssistantCard chip) · StatusPill · hover-revealed
 *   edit/delete actions.
 *
 * The whole row is a `<div role="button">` whose background click opens the
 * detail drawer (`onOpenDetail`). Interactive children — checkbox, status pill,
 * edit, delete — stopPropagation so they never bubble into a drawer-open.
 * Theme tokens only; `<div onClick>` / Arco controls, never a bare <button>.
 */
import { Checkbox, Popconfirm } from '@arco-design/web-react';
import { Delete, Edit } from '@icon-park/react';
import React from 'react';
import { useTranslation } from 'react-i18next';

import type { IRequirement, RequirementStatus } from '@/common/adapter/ipcBridge';
import StatusPill from '../components/StatusPill';

interface RequirementListRowProps {
  item: IRequirement;
  selected: boolean;
  onToggleSelect: (id: number) => void;
  onOpenDetail: (id: number) => void; // row click
  onStatusChange: (id: number, next: RequirementStatus) => void;
  onEdit: (id: number) => void;
  onDelete: (id: number) => void;
}

const stop = (e: React.SyntheticEvent) => e.stopPropagation();

// Keep selected rows readable even when a theme defines primary-light-1 as
// a saturated brand fill. The checkbox remains the strongest selected cue.
const SOFT_SELECTED_ROW_STYLE: React.CSSProperties = {
  background:
    'linear-gradient(rgba(var(--primary-6), 0.055), rgba(var(--primary-6), 0.055)), var(--color-bg-2)',
  borderColor: 'rgba(var(--primary-6), 0.24)',
  boxShadow: '0 0 0 1px rgba(var(--primary-6), 0.08), 0 1px 2px rgba(0, 0, 0, 0.04)',
};

const RequirementListRow: React.FC<RequirementListRowProps> = ({
  item,
  selected,
  onToggleSelect,
  onOpenDetail,
  onStatusChange,
  onEdit,
  onDelete,
}) => {
  const { t } = useTranslation();

  const showCompletionNote =
    (item.status === 'done' || item.status === 'failed') && !!item.completion_note;

  return (
    <div
      role='button'
      tabIndex={0}
      onClick={() => onOpenDetail(item.id)}
      onKeyDown={(e) => {
        if (e.key === 'Enter') {
          e.preventDefault();
          onOpenDetail(item.id);
        }
      }}
      className={[
        'group flex items-center gap-12px rounded-12px border border-solid px-12px py-10px cursor-pointer',
        'transition-all duration-150',
        selected
          ? 'border-[var(--color-border-2)] bg-[var(--color-bg-2)]'
          : 'border-[var(--color-border-2)] bg-[var(--color-bg-2)] hover:border-[var(--color-primary-light-4)] hover:shadow-[0_2px_10px_rgba(0,0,0,0.05)]',
      ].join(' ')}
      style={selected ? SOFT_SELECTED_ROW_STYLE : undefined}
    >
      {/* Checkbox — selection, never opens the drawer */}
      <div className='flex-shrink-0' onClick={stop}>
        <Checkbox checked={selected} onChange={() => onToggleSelect(item.id)} />
      </div>

      {/* `#id` badge — tabular-nums so digits stay aligned column-to-column */}
      <span
        className='flex-shrink-0 text-12px text-[var(--color-text-3)] tabular-nums'
        style={{ fontVariantNumeric: 'tabular-nums' }}
      >
        {`#${item.id}`}
      </span>

      {/* Title + snippet — flexes, truncates gracefully on narrow widths */}
      <div className='flex min-w-0 flex-1 flex-col gap-2px'>
        <span className='truncate text-14px font-medium leading-20px text-[var(--color-text-1)]'>
          {item.title}
        </span>
        {item.content && (
          <span
            className='text-12px leading-18px text-[var(--color-text-3)]'
            style={{
              display: '-webkit-box',
              WebkitLineClamp: 2,
              WebkitBoxOrient: 'vertical',
              overflow: 'hidden',
            }}
          >
            {item.content}
          </span>
        )}
        {showCompletionNote && (
          <span
            className='text-12px leading-18px text-[var(--color-text-4)]'
            style={{
              display: '-webkit-box',
              WebkitLineClamp: 1,
              WebkitBoxOrient: 'vertical',
              overflow: 'hidden',
            }}
          >
            {t('requirements.detail.completionNote')}: {item.completion_note}
          </span>
        )}
      </div>

      {/* Tag chip — mirrors the AssistantCard pill. Hides first on narrow widths. */}
      {item.tag && (
        <span
          className={[
            'hidden flex-shrink-0 sm:inline-flex items-center rounded-[12px] px-8px py-1px text-11px leading-16px max-w-140px',
            'bg-[var(--color-fill-2)] text-[var(--color-text-2)] border border-solid border-[var(--color-border-2)]',
          ].join(' ')}
        >
          <span className='truncate'>{item.tag}</span>
        </span>
      )}

      {/* Status — clickable pill; stopPropagation so it doesn't open the drawer */}
      <div className='flex-shrink-0' onClick={stop}>
        <StatusPill
          status={item.status}
          size='sm'
          onChange={(next) => onStatusChange(item.id, next)}
        />
      </div>

      {/* Hover-revealed actions — quiet icon links, kept off the keyboard tab
          flow until visible to avoid surprising focus jumps on the row. */}
      <div
        className='flex flex-shrink-0 items-center gap-10px opacity-0 group-hover:opacity-100 transition-opacity duration-150'
        onClick={stop}
      >
        <span
          role='button'
          tabIndex={0}
          aria-label={t('requirements.actions.edit')}
          title={t('requirements.actions.edit')}
          onClick={() => onEdit(item.id)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' || e.key === ' ') {
              e.preventDefault();
              onEdit(item.id);
            }
          }}
          className='inline-flex items-center text-[var(--color-text-3)] cursor-pointer hover:text-[rgb(var(--primary-6))] transition-colors'
        >
          <Edit theme='outline' size={15} strokeWidth={3} />
        </span>
        <Popconfirm
          title={t('requirements.actions.deleteConfirm')}
          onOk={() => onDelete(item.id)}
        >
          <span
            role='button'
            tabIndex={0}
            aria-label={t('requirements.actions.delete')}
            title={t('requirements.actions.delete')}
            onKeyDown={(e) => {
              if (e.key === ' ') {
                e.preventDefault();
                (e.currentTarget as HTMLElement).click();
              }
            }}
            className='inline-flex items-center text-[var(--color-text-3)] cursor-pointer hover:text-[rgb(var(--danger-6))] transition-colors'
          >
            <Delete theme='outline' size={15} strokeWidth={3} />
          </span>
        </Popconfirm>
      </div>
    </div>
  );
};

export default RequirementListRow;
