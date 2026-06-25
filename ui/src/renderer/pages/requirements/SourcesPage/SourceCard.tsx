/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * SourceCard — a grid item for the requirements「数据源」section. Mirrors the
 * AssistantCard visual language (rounded-16px bordered surface on bg-2, soft
 * hover, chip badges) but is purely presentational: a leading icon tile, a
 * name + status badge header, a 2-line description clamp, and a status-specific
 * footer (active → count, soon → a quiet "Connect…" affordance, planned → nothing).
 *
 * Theme variables only; `<div role="button">` for the clickable affordance
 * (no bare <button>).
 */
import React from 'react';
import { Tag } from '@arco-design/web-react';
import { Connect } from '@icon-park/react';
import { useTranslation } from 'react-i18next';

export interface SourceCardProps {
  icon: React.ReactNode;
  name: string;
  description: string;
  status: 'active' | 'soon' | 'planned';
  count?: number; // shown only when active
  onConnect?: () => void; // only meaningful for 'soon'
}

const SourceCard: React.FC<SourceCardProps> = ({ icon, name, description, status, count, onConnect }) => {
  const { t } = useTranslation();
  const isActive = status === 'active';

  const statusBadge =
    status === 'active' ? (
      <Tag
        size='small'
        bordered={false}
        className='!flex-shrink-0 !text-10px !leading-14px !px-6px !py-0 !rounded-6px'
        style={{ background: 'rgb(var(--primary-6))', color: 'var(--text-white)' }}
      >
        {t('requirements.source.status.active')}
      </Tag>
    ) : status === 'soon' ? (
      <Tag
        size='small'
        bordered={false}
        className='!flex-shrink-0 !text-10px !leading-14px !px-6px !py-0 !rounded-6px !bg-fill-2 !text-t-secondary'
      >
        {t('requirements.source.status.soon')}
      </Tag>
    ) : (
      <Tag
        size='small'
        bordered={false}
        className='!flex-shrink-0 !text-10px !leading-14px !px-6px !py-0 !rounded-6px !bg-fill-2 !text-t-tertiary'
      >
        {t('requirements.source.status.planned')}
      </Tag>
    );

  return (
    <div
      className={[
        'group relative flex h-full flex-col rounded-16px border border-solid p-14px',
        'transition-all duration-180',
        isActive
          ? ''
          : 'border-[var(--color-border-2)] bg-[var(--color-bg-2)] hover:border-[var(--color-primary-light-4)] hover:shadow-[0_4px_16px_rgba(0,0,0,0.06)]',
      ].join(' ')}
      style={
        isActive
          ? {
              // Follow the theme primary but stay readable on ANY theme: a very
              // soft primary wash (8% over bg-2) — NOT `--color-primary-light-1`,
              // which some saturated themes define as a near-solid fill that
              // destroys text contrast (the bug this fixes). 8% of any primary
              // over bg-2 is always subtle, so text-1/text-3 stay readable. The
              // "active" signal is carried by the colored border + ring + icon
              // tile + badge, not a saturated body.
              background: 'color-mix(in srgb, rgb(var(--primary-6)) 8%, var(--color-bg-2))',
              borderColor: 'rgb(var(--primary-6))',
              boxShadow: '0 0 0 3px rgba(var(--primary-6), 0.12)',
            }
          : undefined
      }
    >
      {/* Header: icon tile + name/badge */}
      <div className='flex items-start gap-10px'>
        <div
          className={[
            'size-36px flex-shrink-0 rounded-10px flex items-center justify-center line-height-0',
            isActive ? 'text-[rgb(var(--primary-6))]' : 'bg-fill-2 text-t-secondary',
          ].join(' ')}
          style={isActive ? { background: 'color-mix(in srgb, rgb(var(--primary-6)) 16%, var(--color-bg-2))' } : undefined}
        >
          {icon}
        </div>
        <div className='min-w-0 flex-1 pt-1px'>
          <div className='flex items-center gap-6px min-w-0'>
            <span className='truncate text-14px font-medium leading-20px text-[var(--color-text-1)]'>{name}</span>
            {statusBadge}
          </div>
        </div>
      </div>

      {/* Description — fixed 2-line clamp so cards stay even-height */}
      <div
        className='mt-10px text-12px leading-18px text-[var(--color-text-3)] min-h-[36px]'
        style={{
          display: '-webkit-box',
          WebkitLineClamp: 2,
          WebkitBoxOrient: 'vertical',
          overflow: 'hidden',
        }}
      >
        {description}
      </div>

      {/* Footer — status-specific */}
      {isActive && typeof count === 'number' && (
        <div className='mt-12px text-12px leading-18px font-medium text-[rgb(var(--primary-6))]'>
          {t('requirements.source.count', { count })}
        </div>
      )}

      {status === 'soon' && (
        <div className='mt-auto pt-12px flex items-center'>
          <span
            role='button'
            tabIndex={0}
            onClick={() => onConnect?.()}
            onKeyDown={(e) => {
              if (e.key === 'Enter' || e.key === ' ') {
                e.preventDefault();
                onConnect?.();
              }
            }}
            className={[
              'inline-flex h-28px items-center gap-6px rounded-8px px-10px',
              'bg-[var(--color-fill-2)] text-12px font-500 leading-none text-[var(--color-text-3)]',
              'cursor-pointer transition-colors duration-180',
              'hover:bg-[var(--color-primary-light-1)] hover:text-[rgb(var(--primary-6))]',
              'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[rgba(var(--primary-6),0.22)]',
            ].join(' ')}
          >
            <span className='inline-flex size-14px flex-shrink-0 items-center justify-center line-height-0'>
              <Connect theme='outline' size={14} strokeWidth={3} />
            </span>
            <span className='leading-16px'>{t('requirements.source.connect')}</span>
          </span>
        </div>
      )}
    </div>
  );
};

export default SourceCard;
