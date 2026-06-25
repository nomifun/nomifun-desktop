/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useMemo } from 'react';
import { Dropdown, Menu } from '@arco-design/web-react';
import { useTranslation } from 'react-i18next';
import type { RequirementStatus } from '@/common/adapter/ipcBridge';

/** All requirement statuses, in a stable display order for the picker. */
const STATUS_ORDER: RequirementStatus[] = [
  'pending',
  'in_progress',
  'needs_review',
  'done',
  'failed',
  'cancelled',
];

/**
 * Per-status accent color, expressed as Arco palette / theme tokens (never hex).
 * `in_progress` follows the theme primary so it doesn't clash with the rose theme,
 * mirroring RequirementStatusTag's special-casing.
 */
const STATUS_ACCENT: Record<RequirementStatus, string> = {
  pending: 'rgb(var(--gray-6))',
  in_progress: 'rgb(var(--primary-6))',
  needs_review: 'rgb(var(--purple-6))',
  done: 'rgb(var(--green-6))',
  failed: 'rgb(var(--red-6))',
  cancelled: 'rgb(var(--orange-6))',
};

interface StatusPillProps {
  status: RequirementStatus;
  /** When provided, the pill becomes a clickable dropdown to pick any of the 6 statuses. */
  onChange?: (next: RequirementStatus) => void;
  size?: 'sm' | 'md';
}

const StatusPill: React.FC<StatusPillProps> = ({ status, onChange, size = 'md' }) => {
  const { t } = useTranslation();

  const accent = STATUS_ACCENT[status];
  const interactive = typeof onChange === 'function';

  const sizing =
    size === 'sm'
      ? { pad: 'px-7px py-1px gap-5px text-11px leading-16px', dot: 5 }
      : { pad: 'px-9px py-2px gap-6px text-12px leading-18px', dot: 6 };

  const pill = (
    <span
      className={`inline-flex items-center rounded-full font-500 select-none ${sizing.pad}`}
      style={{
        // Soft tint of the accent for the background, solid accent for text — restrained, clean.
        backgroundColor: `color-mix(in srgb, ${accent} 14%, transparent)`,
        color: accent,
      }}
    >
      <span
        className='inline-block rounded-full flex-shrink-0'
        style={{ width: sizing.dot, height: sizing.dot, backgroundColor: accent }}
      />
      <span>{t(`requirements.status.${status}`)}</span>
    </span>
  );

  const droplist = useMemo(() => {
    if (!interactive) return null;
    return (
      <Menu
        onClickMenuItem={(key) => {
          onChange?.(key as RequirementStatus);
        }}
      >
        {STATUS_ORDER.map((s) => (
          <Menu.Item key={s}>
            <span className='inline-flex items-center gap-8px min-w-120px'>
              <span
                className='inline-block w-6px h-6px rounded-full flex-shrink-0'
                style={{ backgroundColor: STATUS_ACCENT[s] }}
              />
              <span>{t(`requirements.status.${s}`)}</span>
            </span>
          </Menu.Item>
        ))}
      </Menu>
    );
  }, [interactive, onChange, t]);

  if (!interactive || !droplist) {
    return pill;
  }

  return (
    <Dropdown droplist={droplist} trigger='click' position='bl' getPopupContainer={() => document.body}>
      <div
        role='button'
        tabIndex={0}
        className='inline-flex cursor-pointer outline-none'
        onClick={(e) => {
          // Keep clicks from bubbling to a surrounding clickable row.
          e.stopPropagation();
        }}
        onKeyDown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            e.stopPropagation();
            (e.currentTarget as HTMLDivElement).click();
          }
        }}
      >
        {pill}
      </div>
    </Dropdown>
  );
};

export default StatusPill;
