/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { NotebookOne } from '@icon-park/react';
import type { ICompanionDayDigest } from '@/common/adapter/ipcBridge';
import { formatClock, parseDigestHighlights } from './historyFormat';

interface DigestCardProps {
  digest: ICompanionDayDigest;
}

/**
 * One archived session-window digest: the LLM's narrative summary plus its mood
 * and topic tags. A summary layer above the raw messages — never a substitute
 * for them, and only present when 会话归档 was enabled.
 */
const DigestCard: React.FC<DigestCardProps> = ({ digest }) => {
  const { t } = useTranslation();
  const { topics, mood } = parseDigestHighlights(digest.highlights);

  return (
    <div className='flex flex-col gap-8px rd-10px bg-fill-2 px-12px py-10px'>
      <div className='flex items-center gap-6px'>
        <span className='flex items-center text-primary-6'>
          <NotebookOne theme='outline' size='14' fill='currentColor' strokeWidth={3} />
        </span>
        <span className='text-13px font-500 text-t-primary'>
          {t('nomi.history.digestTitle', { defaultValue: '当天日记' })}
        </span>
        <span className='ml-auto text-11px text-t-tertiary'>
          {formatClock(digest.started_at)} · {t('nomi.history.messageCount', {
            defaultValue: '{{count}} 条',
            count: digest.message_count,
          })}
        </span>
      </div>
      <div className='whitespace-pre-wrap text-13px leading-21px text-t-secondary'>{digest.digest || '—'}</div>
      {(mood || topics.length > 0) && (
        <div className='flex flex-wrap items-center gap-6px'>
          {mood && (
            <span className='rd-full bg-primary-1 px-8px py-2px text-11px text-primary-6'>{mood}</span>
          )}
          {topics.map((topic) => (
            <span key={topic} className='rd-full bg-[var(--color-bg-2)] px-8px py-2px text-11px text-t-tertiary'>
              {topic}
            </span>
          ))}
        </div>
      )}
    </div>
  );
};

export default DigestCard;
