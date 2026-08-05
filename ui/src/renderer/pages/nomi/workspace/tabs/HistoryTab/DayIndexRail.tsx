/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import classNames from 'classnames';
import { formatDayKey, formatDayKeyShort, isToday, isYesterday, type DayKey } from './historyFormat';
import type { HistoryDay } from './useChatHistory';

interface DayIndexRailProps {
  days: HistoryDay[];
  selectedDay: DayKey | null;
  onSelect: (day: DayKey) => void;
  hasMore: boolean;
  loadingMore: boolean;
  onLoadMore: () => void;
  entryCount: number;
  oldestDay: DayKey | null;
  /** A previous 「加载更早」 failed — say so instead of silently stopping. */
  loadMoreFailed?: boolean;
}

const DayLabel: React.FC<{ day: DayKey }> = ({ day }) => {
  const { t } = useTranslation();
  if (isToday(day)) return <>{t('nomi.history.today', { defaultValue: '今天' })}</>;
  if (isYesterday(day)) return <>{t('nomi.history.yesterday', { defaultValue: '昨天' })}</>;
  return <>{formatDayKeyShort(day)}</>;
};

/**
 * The day index. Derived client-side from the loaded message window, so it grows
 * only as far back as the user has asked: 「加载更早」 is an explicit action at the
 * bottom, and the footnote states exactly how far the index currently reaches.
 */
const DayIndexRail: React.FC<DayIndexRailProps> = ({
  days,
  selectedDay,
  onSelect,
  hasMore,
  loadingMore,
  onLoadMore,
  entryCount,
  oldestDay,
  loadMoreFailed = false,
}) => {
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
              {entry.digests.length > 0 && (
                <span
                  className='h-5px w-5px shrink-0 rd-full bg-primary-6'
                  title={t('nomi.history.digestMark', { defaultValue: '这一天有日记' })}
                />
              )}
              <span className={classNames('shrink-0 text-11px', selected ? undefined : 'text-t-tertiary')}>
                {entry.entries.length}
              </span>
            </div>
          );
        })}
        {hasMore && (
          <div
            role='button'
            tabIndex={0}
            aria-disabled={loadingMore}
            onClick={() => !loadingMore && onLoadMore()}
            onKeyDown={(event) => {
              if ((event.key === 'Enter' || event.key === ' ') && !loadingMore) {
                event.preventDefault();
                onLoadMore();
              }
            }}
            className={classNames(
              'cursor-pointer px-10px py-8px text-center text-12px text-primary-6 outline-none transition-colors',
              days.length > 0 ? 'border-t border-t-solid border-t-[var(--color-border-2)]' : undefined,
              loadingMore ? 'cursor-default opacity-60' : 'hover:bg-fill-2 active:bg-fill-3'
            )}
          >
            {loadingMore
              ? t('nomi.history.loadingEarlier', { defaultValue: '正在加载…' })
              : loadMoreFailed
                ? t('nomi.history.loadEarlierFailed', { defaultValue: '加载失败，点此重试' })
                : t('nomi.history.loadEarlier', { defaultValue: '加载更早' })}
          </div>
        )}
      </div>
      <div className='text-11px leading-16px text-t-tertiary'>
        {hasMore && oldestDay
          ? t('nomi.history.partialHint', {
              defaultValue: '已读取 {{count}} 条可读消息，日期索引只到 {{day}}。更早的日期需要点「加载更早」。',
              count: entryCount,
              day: formatDayKey(oldestDay),
            })
          : t('nomi.history.allLoaded', {
              defaultValue: '已加载全部历史（{{count}} 条可读消息）。',
              count: entryCount,
            })}
      </div>
    </div>
  );
};

export default DayIndexRail;
