/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Spin } from '@arco-design/web-react';
import { NomiSettingSection } from '@/renderer/components/base/NomiSettingLayout';
import SegmentedTabs from '@/renderer/components/base/SegmentedTabs';
import type { WorkspaceTabProps } from '../../types';
import DayIndexRail from './DayIndexRail';
import DayReader from './DayReader';
import HistoryEmptyState from './HistoryEmptyState';
import OnThisDayPanel from './OnThisDayPanel';
import { useChatHistory } from './useChatHistory';
import type { DayKey } from './historyFormat';

type Mode = 'byDay' | 'onThisDay';

const isMode = (value: string): value is Mode => value === 'byDay' || value === 'onThisDay';

/**
 * 聊天历史 — this companion's single long-lived conversation, read by day.
 *
 * Read-only: the session is resolved, never minted. The day index is derived
 * client-side from a keyset-paged message window (there is no day-index endpoint),
 * so the rail states how far back it currently reaches and 「加载更早」 extends it.
 * Day digests, when 会话归档 produced any, ride above the messages as a summary.
 */
const HistoryTab: React.FC<WorkspaceTabProps> = ({ companionId, companion, onAttentionChange }) => {
  const { t } = useTranslation();
  const history = useChatHistory(companionId);
  const [mode, setMode] = useState<Mode>('byDay');
  const [selectedDay, setSelectedDay] = useState<DayKey | null>(null);

  // A history reader never demands attention — it is always a lookback surface.
  useEffect(() => {
    onAttentionChange?.(false);
  }, [onAttentionChange]);

  // Keep the selection valid across companion switches and 加载更早 (which only
  // appends OLDER days, so the newest-day default survives a load).
  useEffect(() => {
    if (history.days.length === 0) {
      setSelectedDay((prev) => (prev === null ? prev : null));
      return;
    }
    setSelectedDay((prev) => (prev && history.days.some((entry) => entry.day === prev) ? prev : history.days[0].day));
  }, [history.days]);

  const companionName = companion.profile?.name ?? t('nomi.history.roleCompanion', { defaultValue: '伙伴' });
  const selected = history.days.find((entry) => entry.day === selectedDay) ?? null;

  const modeItems = useMemo(
    () => [
      { key: 'byDay', label: t('nomi.history.modeByDay', { defaultValue: '按天' }) },
      { key: 'onThisDay', label: t('nomi.archive.onThisDay', { defaultValue: '去年今日' }) },
    ],
    [t]
  );

  const body = (() => {
    if (mode === 'onThisDay') return <OnThisDayPanel companionId={companionId} />;

    if (history.loading) {
      return (
        <div className='flex justify-center py-40px'>
          <Spin />
        </div>
      );
    }

    if (history.failed && history.days.length === 0) {
      return (
        <HistoryEmptyState
          title={t('nomi.history.loadFailedTitle', { defaultValue: '历史加载失败' })}
          description={t('nomi.history.loadFailedHint', {
            defaultValue: '没能读到这个伙伴的会话记录，可能是后端暂时不可用。稍后再试一次。',
          })}
          onRetry={history.retry}
        />
      );
    }

    if (history.conversationId == null || (history.days.length === 0 && !history.hasMore)) {
      return (
        <HistoryEmptyState
          title={t('nomi.history.emptyTitle', { defaultValue: '还没有聊天记录' })}
          description={t('nomi.history.emptyHint', {
            defaultValue: '和这个伙伴聊过第一句之后，对话会按天出现在这里。',
          })}
        />
      );
    }

    // A conversation with older windows left, whose newest window held nothing a
    // human re-reads (permission prompts, status pings). Carry 「加载更早」 into the
    // zero-state so this is never a dead end with no way forward.
    if (history.days.length === 0) {
      return (
        <HistoryEmptyState
          title={t('nomi.history.noReadableTitle', { defaultValue: '这一段没有可阅读的内容' })}
          description={t('nomi.history.noReadableHint', {
            defaultValue: '最近读到的记录里只有系统消息。继续往前翻可以找到真正的对话。',
          })}
          onRetry={history.loadMore}
          retryLabel={
            history.loadingMore
              ? t('nomi.history.loadingEarlier', { defaultValue: '正在加载…' })
              : t('nomi.history.loadEarlier', { defaultValue: '加载更早' })
          }
        />
      );
    }

    return (
      <div className='flex min-w-0 items-start gap-16px max-[760px]:flex-col'>
        <div
          className='sticky top-0 w-200px shrink-0 overflow-y-auto max-[760px]:static max-[760px]:w-full max-[760px]:overflow-visible'
          style={{ maxHeight: 'calc(100vh - 200px)' }}
        >
          <DayIndexRail
            days={history.days}
            selectedDay={selectedDay}
            onSelect={setSelectedDay}
            hasMore={history.hasMore}
            loadingMore={history.loadingMore}
            onLoadMore={history.loadMore}
            entryCount={history.entryCount}
            oldestDay={history.oldestDay}
            loadMoreFailed={history.failed}
          />
        </div>
        <div className='min-w-0 flex-1'>
          {selected ? (
            <DayReader
              day={selected}
              companionName={companionName}
              partial={history.hasMore && selected.day === history.oldestDay}
            />
          ) : (
            <div className='py-40px text-center text-13px text-t-tertiary'>
              {t('nomi.history.selectDay', { defaultValue: '从左侧选择一天开始阅读。' })}
            </div>
          )}
        </div>
      </div>
    );
  })();

  return (
    <div className='flex flex-col gap-16px py-8px'>
      <NomiSettingSection
        title={t('nomi.history.sectionTitle', { defaultValue: '聊天历史' })}
        description={t('nomi.history.sectionDesc', {
          defaultValue:
            '这个伙伴的对话按本地日期分天阅读；开启「会话归档」（全局设置，对所有伙伴生效，默认关闭）后，当天的日记会显示在消息上方。',
        })}
        action={
          <SegmentedTabs
            items={modeItems}
            activeKey={mode}
            size='sm'
            onChange={(key) => {
              if (isMode(key)) setMode(key);
            }}
          />
        }
      >
        {body}
      </NomiSettingSection>
    </div>
  );
};

export default HistoryTab;
