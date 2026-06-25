/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';

import PathText from '@renderer/components/base/PathText';

export interface BindTargetOptionRowProps {
  /** Primary label — the terminal / conversation name (id fallback at call site). */
  title: string;
  /** Right-aligned dimmed badge — terminal status or conversation backend/type. */
  badge?: React.ReactNode;
  /** Working directory / workspace path, middle-truncated on line 2 (project tail kept). */
  path?: string;
  /** Short id rendered as `#N`, kept fully visible on line 2. */
  idLabel: string;
  /** Marks the row as the binding the task being edited currently uses. */
  isCurrent?: boolean;
}

/**
 * Two-line option row shared by the cron task-binding dropdowns (terminal and
 * "specified conversation") so both pickers surface the same disambiguating
 * info at a glance:
 * - Line 1: name + a dimmed status/type badge (+ a "current" chip in edit mode).
 * - Line 2: the working path (middle-truncated via {@link PathText} so the
 *   project folder stays readable in tight widths) followed by the `#id`.
 */
const BindTargetOptionRow: React.FC<BindTargetOptionRowProps> = ({ title, badge, path, idLabel, isCurrent }) => {
  const { t } = useTranslation();
  return (
    <div className='flex min-w-0 flex-col gap-2px py-2px leading-tight'>
      <div className='flex min-w-0 items-center gap-8px'>
        <span className='min-w-0 truncate text-14px text-t-primary'>{title}</span>
        {isCurrent && (
          <span className='shrink-0 rounded-4px bg-primary-1 px-4px text-11px text-primary-6'>
            {t('cron.page.form.optionCurrent', { defaultValue: '当前' })}
          </span>
        )}
        {badge != null && badge !== '' && <span className='ml-auto shrink-0 text-12px text-t-tertiary'>{badge}</span>}
      </div>
      <div className='flex min-w-0 items-center gap-6px text-11px text-t-tertiary'>
        {path ? (
          <>
            <PathText path={path} className='min-w-0 flex-1' />
            <span className='shrink-0'>·</span>
            <span className='shrink-0'>{idLabel}</span>
          </>
        ) : (
          <span className='shrink-0'>{idLabel}</span>
        )}
      </div>
    </div>
  );
};

export default BindTargetOptionRow;
