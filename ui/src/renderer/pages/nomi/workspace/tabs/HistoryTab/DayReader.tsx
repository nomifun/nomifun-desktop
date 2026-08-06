/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import classNames from 'classnames';
import { Spin } from '@arco-design/web-react';
import DigestCard from './DigestCard';
import ReaderPanel from './ReaderPanel';
import { formatClock, formatDayKey, isToday, isYesterday } from './historyFormat';
import type { HistoryEntry } from './historyFormat';
import type { DayContent, HistoryDay } from './useChatHistory';

interface DayReaderProps {
  day: HistoryDay;
  content: DayContent;
  /** Display name for the companion side of the transcript. */
  companionName: string;
}

/** One reader line. Plain text only — the full chat renderer stays out of here. */
const EntryLine: React.FC<{ entry: HistoryEntry; companionName: string }> = ({ entry, companionName }) => {
  const { t } = useTranslation();

  if (entry.kind === 'tool' || entry.kind === 'note') {
    const label =
      entry.kind === 'tool'
        ? t('nomi.history.toolCall', { defaultValue: '使用了工具' })
        : t('nomi.history.systemNote', { defaultValue: '系统提示' });
    return (
      <div className='flex items-baseline gap-8px text-12px leading-18px text-t-tertiary'>
        <span className='shrink-0 tabular-nums'>{formatClock(entry.createdAt)}</span>
        <span className='min-w-0 flex-1 truncate'>
          {label}
          {entry.text ? ` · ${entry.text}` : ''}
        </span>
      </div>
    );
  }

  const isUser = entry.role === 'user';
  const speaker = isUser ? t('nomi.history.roleUser', { defaultValue: '我' }) : companionName;

  return (
    <div className={classNames('flex flex-col gap-4px rd-10px px-12px py-9px', isUser ? 'bg-fill-2' : undefined)}>
      <div className='flex items-baseline gap-8px'>
        <span
          className={classNames('text-12px font-500', isUser ? 'text-t-secondary' : 'text-primary-6')}
        >
          {speaker}
        </span>
        {entry.kind === 'thinking' && (
          <span className='text-11px text-t-tertiary'>
            {t('nomi.history.thinking', { defaultValue: '思考' })}
          </span>
        )}
        <span className='ml-auto shrink-0 text-11px tabular-nums text-t-tertiary'>
          {formatClock(entry.createdAt)}
        </span>
      </div>
      <div
        className={classNames(
          'whitespace-pre-wrap break-words text-13px leading-21px',
          entry.kind === 'thinking' ? 'text-t-tertiary' : 'text-t-primary'
        )}
      >
        {entry.text}
      </div>
    </div>
  );
};

/**
 * The selected day, in order: its digest (when 会话归档 produced one) as a summary
 * above the raw messages, then the messages themselves.
 *
 * The header count comes from the day index, so the rail and the reader can never
 * disagree; rendering fewer LINES than that is normal — a tool call with no name
 * or an empty status ping carries nothing a human re-reads.
 */
const DayReader: React.FC<DayReaderProps> = ({ day, content, companionName }) => {
  const { t } = useTranslation();
  const relative = isToday(day.day)
    ? t('nomi.history.today', { defaultValue: '今天' })
    : isYesterday(day.day)
      ? t('nomi.history.yesterday', { defaultValue: '昨天' })
      : null;

  const body = (() => {
    if (content.loading) {
      return (
        <div className='flex justify-center py-32px'>
          <Spin />
        </div>
      );
    }
    if (content.failed) {
      return (
        <div
          role='button'
          tabIndex={0}
          onClick={content.retry}
          onKeyDown={(event) => {
            if (event.key === 'Enter' || event.key === ' ') {
              event.preventDefault();
              content.retry();
            }
          }}
          className='cursor-pointer py-24px text-center text-13px text-primary-6 outline-none'
        >
          {t('nomi.history.dayLoadFailed', { defaultValue: '这一天没读出来，点此重试' })}
        </div>
      );
    }
    if (content.entries.length === 0) {
      return (
        <div className='py-24px text-center text-13px text-t-tertiary'>
          {t('nomi.history.emptyDay', { defaultValue: '这一天没有可显示的消息。' })}
        </div>
      );
    }
    return (
      <div className='flex flex-col gap-8px'>
        {content.entries.map((entry) => (
          <EntryLine key={entry.key} entry={entry} companionName={companionName} />
        ))}
      </div>
    );
  })();

  return (
    <ReaderPanel
      header={
        <>
          <span className='text-15px font-600 leading-22px text-t-primary'>{formatDayKey(day.day)}</span>
          {relative && (
            <span className='rd-full bg-primary-1 px-8px py-1px text-11px text-primary-6'>{relative}</span>
          )}
          <span className='ml-auto shrink-0 text-11px text-t-tertiary'>
            {t('nomi.history.messageCount', { defaultValue: '{{count}} 条', count: day.messageCount })}
          </span>
        </>
      }
    >
      {content.digests.map((digest) => (
        <DigestCard key={digest.session_window_id} digest={digest} />
      ))}
      {content.truncated && (
        <div className='text-11px leading-16px text-t-tertiary'>
          {t('nomi.history.dayTruncatedHint', {
            defaultValue: '这一天太长了，只显示最早的 {{count}} 条。',
            count: content.entries.length,
          })}
        </div>
      )}
      {body}
    </ReaderPanel>
  );
};

export default DayReader;
