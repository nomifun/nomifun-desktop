/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * WorkspaceEmptyState — the centered zero-state for the requirements workspace
 * list. A soft circular icon badge + title + one-line description + a single
 * primary CTA that calls `onCreate`. Theme tokens only; intentionally calm
 * (not garish) so it reads as an invitation rather than an error.
 */
import { Button } from '@arco-design/web-react';
import { ListAdd } from '@icon-park/react';
import React from 'react';
import { useTranslation } from 'react-i18next';

interface WorkspaceEmptyStateProps {
  onCreate: () => void;
}

const WorkspaceEmptyState: React.FC<WorkspaceEmptyStateProps> = ({ onCreate }) => {
  const { t } = useTranslation();

  return (
    <div className='flex flex-col items-center justify-center gap-14px px-24px py-64px text-center'>
      <div
        className='flex items-center justify-center rounded-full'
        style={{
          width: 72,
          height: 72,
          background: 'var(--color-fill-2)',
          color: 'rgb(var(--primary-6))',
        }}
      >
        <ListAdd theme='outline' size={32} strokeWidth={3} />
      </div>
      <div className='flex flex-col items-center gap-6px'>
        <span className='text-16px font-medium text-[var(--color-text-1)]'>
          {t('requirements.emptyState.title')}
        </span>
        <span className='max-w-360px text-13px leading-20px text-[var(--color-text-3)]'>
          {t('requirements.emptyState.desc')}
        </span>
      </div>
      <Button type='primary' shape='round' className='mt-4px' onClick={onCreate}>
        {t('requirements.emptyState.cta')}
      </Button>
    </div>
  );
};

export default WorkspaceEmptyState;
