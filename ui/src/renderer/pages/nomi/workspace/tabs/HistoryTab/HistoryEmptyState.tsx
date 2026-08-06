/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { Comment } from '@icon-park/react';

interface HistoryEmptyStateProps {
  /**
   * Optional recovery action — retrying a failed read or reaching further back.
   * Never a "start chatting" CTA: that would mint a session from a reader.
   */
  onRetry?: () => void;
  title: string;
  description: string;
}

/**
 * Zero-state for the history reader. There is intentionally no "start a chat"
 * CTA: minting a session from a history reader would write to the backend (and
 * 400 for a companion with no model configured). History simply appears after
 * the first real conversation.
 */
const HistoryEmptyState: React.FC<HistoryEmptyStateProps> = ({ onRetry, title, description }) => {
  const { t } = useTranslation();

  return (
    <div className='flex flex-col items-center justify-center gap-14px px-24px py-64px text-center'>
      <span className='flex h-72px w-72px items-center justify-center rd-full bg-fill-2 text-primary-6'>
        <Comment theme='outline' size='30' fill='currentColor' strokeWidth={3} />
      </span>
      <span className='text-16px font-500 text-t-primary'>{title}</span>
      <span className='max-w-360px text-13px leading-20px text-t-tertiary'>{description}</span>
      {onRetry && (
        <div
          role='button'
          tabIndex={0}
          onClick={onRetry}
          onKeyDown={(event) => {
            if (event.key === 'Enter' || event.key === ' ') {
              event.preventDefault();
              onRetry();
            }
          }}
          className='mt-2px cursor-pointer rd-full bg-[rgba(var(--primary-6),0.12)] px-18px py-9px text-13px font-700 text-[var(--color-text-1)] shadow-[0_6px_18px_rgba(var(--primary-6),0.14)] outline-none transition-colors hover:bg-[rgba(var(--primary-6),0.18)]'
        >
          {t('nomi.history.retry', { defaultValue: '重试' })}
        </div>
      )}
    </div>
  );
};

export default HistoryEmptyState;
