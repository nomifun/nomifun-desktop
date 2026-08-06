/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import classNames from 'classnames';
import { formatDayKeyShort, isToday, isYesterday, type DayKey } from './historyFormat';
import type { HistoryDay } from './useChatHistory';

interface DayIndexRailProps {
  days: HistoryDay[];
  selectedDay: DayKey | null;
  onSelect: (day: DayKey) => void;
  /** Visible messages across every day — the index is complete, so this is total. */
  messageCount: number;
}

const DayLabel: React.FC<{ day: DayKey }> = ({ day }) => {
  const { t } = useTranslation();
  if (isToday(day)) return <>{t('nomi.history.today', { defaultValue: '今天' })}</>;
  if (isYesterday(day)) return <>{t('nomi.history.yesterday', { defaultValue: '昨天' })}</>;
  return <>{formatDayKeyShort(day)}</>;
};

/**
 * The day index. Read whole from the server, so it reaches all the way back with
 * no 「加载更早」 and no footnote about how far it currently goes: every day this
 * companion has history on is already in this list.
 */
const DayIndexRail: React.FC<DayIndexRailProps> = ({ days, selectedDay, onSelect, messageCount }) => {
  const { t } = useTranslation();

  return (
    <div className='flex flex-col gap-8px'>
      <div className='text-14px font-600 leading-20px text-t-primary'>
        {t('nomi.history.dayIndexTitle', { defaultValue: '日期' })}
      </div>
      <div className='flex flex-col overflow-hidden rd-10px border border-solid border-[var(--color-border-2)]'>
        {days.map((entry, index) => {
          const selected = entry.day === selectedDay;
          return (
            <div
              key={entry.day}
              role='button'
              tabIndex={0}
              onClick={() => onSelect(entry.day)}
              onKeyDown={(event) => {
                if (event.key === 'Enter' || event.key === ' ') {
                  event.preventDefault();
                  onSelect(entry.day);
                }
              }}
              className={classNames(
                'flex cursor-pointer items-center gap-6px px-10px py-8px text-13px outline-none transition-colors',
                index === 0 ? undefined : 'border-t border-t-solid border-t-[var(--color-border-2)]',
                selected ? '!bg-primary-1 !text-primary-6' : 'text-t-primary hover:bg-fill-2 active:bg-fill-3'
              )}
            >
              <span className='min-w-0 flex-1 truncate font-500'>
                <DayLabel day={entry.day} />
              </span>
              {entry.hasDigest && (
                <span
                  className='h-5px w-5px shrink-0 rd-full bg-primary-6'
                  title={t('nomi.history.digestMark', { defaultValue: '这一天有日记' })}
                />
              )}
              <span className={classNames('shrink-0 text-11px', selected ? undefined : 'text-t-tertiary')}>
                {entry.messageCount}
              </span>
            </div>
          );
        })}
      </div>
      <div className='text-11px leading-16px text-t-tertiary'>
        {t('nomi.history.dayIndexSummary', {
          defaultValue: '共 {{days}} 天 · {{count}} 条消息，这就是全部。',
          days: days.length,
          count: messageCount,
        })}
      </div>
    </div>
  );
};

export default DayIndexRail;
