/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { History } from '@icon-park/react';

/**
 * RunHistory — the 「Run 历史」section of the orchestration page. Orchestration
 * run execution is a P1 feature; for now this is a calm empty-state placeholder
 * so the shell reads complete. Task 12+ replaces it with the real run list.
 */
const RunHistory: React.FC = () => {
  const { t } = useTranslation();
  return (
    <div className='w-full'>
      <div className='mb-16px'>
        <div className='text-18px font-600 text-t-primary leading-tight'>{t('orchestrator.runHistory.title')}</div>
      </div>
      <div className='rd-12px bg-1 px-24px py-48px flex flex-col items-center justify-center text-center'>
        <span className='size-48px rd-14px bg-fill-2 text-t-tertiary flex items-center justify-center mb-14px'>
          <History theme='outline' size='24' strokeWidth={3} />
        </span>
        <div className='text-15px font-600 text-t-primary'>{t('orchestrator.runHistory.emptyTitle')}</div>
        <div className='mt-6px text-12px leading-18px text-t-tertiary max-w-320px'>
          {t('orchestrator.runHistory.emptyDesc')}
        </div>
      </div>
    </div>
  );
};

export default RunHistory;
