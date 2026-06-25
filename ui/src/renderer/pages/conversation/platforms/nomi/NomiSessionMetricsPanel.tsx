/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { TChatConversation, TokenUsageData } from '@/common/config/storage';
import { addEventListener } from '@/renderer/utils/emitter';
import { Empty } from '@arco-design/web-react';
import { ChartHistogram, Dashboard, Lightning, Time } from '@icon-park/react';
import React, { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  calculateCacheHitRatePercent,
  calculateContextUsagePercent,
  calculateContextUsageSegments,
  formatPercent,
  formatTokenCount,
  formatTurnDuration,
} from './turnMetrics';

const fallback = '—';

const formatFullNumber = (value?: number): string => (typeof value === 'number' ? value.toLocaleString() : fallback);
const formatCompactToken = (value?: number): string => (typeof value === 'number' ? formatTokenCount(value) : fallback);

const getPersistedUsage = (conversation: TChatConversation): TokenUsageData | null =>
  ((conversation.extra as { last_token_usage?: TokenUsageData } | undefined)?.last_token_usage ?? null);

const hasUsageData = (usage: TokenUsageData | null): usage is TokenUsageData =>
  Boolean(
    usage &&
      ((usage.total_tokens ?? 0) > 0 ||
        (usage.input_tokens ?? 0) > 0 ||
        (usage.output_tokens ?? 0) > 0 ||
        (usage.context_tokens ?? 0) > 0 ||
        (usage.cache_read_tokens ?? 0) > 0 ||
        (usage.cache_creation_tokens ?? 0) > 0)
  );

const formatSessionSpan = (createdAt?: number, modifiedAt?: number): string => {
  if (!createdAt || !modifiedAt || modifiedAt <= createdAt) return fallback;
  return formatTurnDuration(modifiedAt - createdAt);
};

const getContextTone = (percent: number | null): { labelKey: string; color: string } => {
  if (percent == null) {
    return { labelKey: 'conversation.sessionMetrics.status.unknown', color: 'var(--color-text-3)' };
  }
  if (percent >= 90) {
    return { labelKey: 'conversation.sessionMetrics.status.tight', color: 'rgb(var(--danger-6))' };
  }
  if (percent >= 70) {
    return { labelKey: 'conversation.sessionMetrics.status.warming', color: 'rgb(var(--warning-6))' };
  }
  return { labelKey: 'conversation.sessionMetrics.status.healthy', color: 'rgb(var(--success-6))' };
};

const MetricTile: React.FC<{
  label: string;
  value: string;
  caption?: string;
  icon?: React.ReactNode;
}> = ({ label, value, caption, icon }) => {
  return (
    <div className='rounded-8px border border-solid border-[var(--color-border-2)] bg-fill-1 px-10px py-9px min-w-0'>
      <div className='flex items-center justify-between gap-8px text-[11px] text-t-tertiary leading-16px'>
        <span className='truncate'>{label}</span>
        {icon && <span className='shrink-0 text-t-tertiary'>{icon}</span>}
      </div>
      <div className='mt-4px text-18px leading-24px font-600 text-t-primary tabular-nums truncate'>{value}</div>
      {caption && <div className='mt-2px text-[11px] leading-15px text-t-tertiary truncate'>{caption}</div>}
    </div>
  );
};

const DistributionLegend: React.FC<{
  color: string;
  label: string;
  value: string;
  percent: number;
}> = ({ color, label, value, percent }) => (
  <div className='flex items-center justify-between gap-8px text-11px leading-16px'>
    <span className='flex items-center gap-6px min-w-0 text-t-secondary'>
      <span className='size-7px rounded-full shrink-0' style={{ background: color }} />
      <span className='truncate'>{label}</span>
    </span>
    <span className='tabular-nums text-t-tertiary shrink-0'>
      {value} · {percent}%
    </span>
  </div>
);

const NomiSessionMetricsPanel: React.FC<{ conversation: TChatConversation }> = ({ conversation }) => {
  const { t } = useTranslation();
  const [usage, setUsage] = useState<TokenUsageData | null>(() => getPersistedUsage(conversation));

  useEffect(() => {
    setUsage(getPersistedUsage(conversation));
  }, [conversation]);

  useEffect(() => {
    return addEventListener('nomi.usage.updated', ({ conversation_id, tokenUsage }) => {
      if (conversation_id === conversation.id) {
        setUsage(tokenUsage);
      }
    });
  }, [conversation.id]);

  const contextPercent = calculateContextUsagePercent(usage?.context_tokens, usage?.context_window);
  const contextSegments = calculateContextUsageSegments({
    contextTokens: usage?.context_tokens,
    contextWindow: usage?.context_window,
    cacheReadTokens: usage?.cache_read_tokens,
  });
  const cachePercent = calculateCacheHitRatePercent({
    inputTokens: usage?.input_tokens,
    cacheReadTokens: usage?.cache_read_tokens,
  });
  const contextTone = getContextTone(contextPercent);
  const cachedContextColor = 'rgb(var(--primary-6))';
  const freshContextColor = contextTone.color;
  const remainingContextColor = 'var(--color-fill-3)';
  const sessionSpan = useMemo(
    () => formatSessionSpan(conversation.created_at, conversation.modified_at),
    [conversation.created_at, conversation.modified_at]
  );

  if (!hasUsageData(usage)) {
    return (
      <div className='size-full flex items-center justify-center px-16px'>
        <Empty
          description={
            <div className='text-center'>
              <div className='text-14px font-600 text-t-secondary'>
                {t('conversation.sessionMetrics.emptyTitle')}
              </div>
              <div className='mt-4px text-12px leading-18px text-t-tertiary'>
                {t('conversation.sessionMetrics.emptyDesc')}
              </div>
            </div>
          }
        />
      </div>
    );
  }

  return (
    <div className='w-full p-12px pb-16px box-border text-t-primary' data-testid='nomi-session-metrics-panel'>
      <div className='mb-12px'>
        <div className='text-13px font-600 leading-20px'>{t('conversation.sessionMetrics.title')}</div>
        <div className='text-11px text-t-tertiary leading-16px'>{t('conversation.sessionMetrics.subtitle')}</div>
        <div className='mt-8px rounded-6px border border-solid border-[rgb(var(--warning-3))] bg-[rgba(var(--warning-1),0.72)] px-8px py-6px text-11px leading-16px text-[rgb(var(--warning-8))]'>
          {t('conversation.sessionMetrics.notice')}
        </div>
      </div>

      <div className='grid grid-cols-2 gap-8px'>
        <MetricTile
          label={t('conversation.sessionMetrics.elapsed')}
          value={typeof usage.elapsed_ms === 'number' ? formatTurnDuration(usage.elapsed_ms) : fallback}
          caption={t('conversation.sessionMetrics.elapsedCaption')}
          icon={<Time theme='outline' size='14' />}
        />
        <MetricTile
          label={t('conversation.sessionMetrics.sessionSpan')}
          value={sessionSpan}
          caption={t('conversation.sessionMetrics.sessionSpanCaption')}
          icon={<Dashboard theme='outline' size='14' />}
        />
      </div>

      <section className='mt-12px rounded-8px border border-solid border-[var(--color-border-2)] bg-fill-1 p-10px'>
        <div className='flex items-center justify-between gap-8px'>
          <div>
            <div className='text-12px font-600 leading-18px'>{t('conversation.sessionMetrics.contextTitle')}</div>
            <div className='text-11px text-t-tertiary leading-16px'>
              {formatFullNumber(usage.context_tokens)} / {formatFullNumber(usage.context_window)}
            </div>
          </div>
          <div className='text-right'>
            <div className='text-18px leading-24px font-600 tabular-nums' style={{ color: contextTone.color }}>
              {formatPercent(contextPercent)}
            </div>
            <div className='text-11px text-t-tertiary leading-16px'>{t(contextTone.labelKey)}</div>
          </div>
        </div>
        {contextSegments && (
          <>
            <div className='mt-9px h-8px rounded-full bg-fill-3 overflow-hidden flex'>
              {contextSegments.cachedTokens > 0 && (
                <div
                  className='h-full transition-width duration-200'
                  style={{ width: `${contextSegments.cachedPercent}%`, background: cachedContextColor }}
                />
              )}
              {contextSegments.freshTokens > 0 && (
                <div
                  className='h-full transition-width duration-200'
                  style={{ width: `${contextSegments.freshPercent}%`, background: freshContextColor }}
                />
              )}
              {contextSegments.remainingTokens > 0 && (
                <div
                  className='h-full transition-width duration-200'
                  style={{ width: `${contextSegments.remainingPercent}%`, background: remainingContextColor }}
                />
              )}
            </div>
            <div className='mt-8px space-y-4px'>
              <DistributionLegend
                color={cachedContextColor}
                label={t('conversation.sessionMetrics.contextCached')}
                value={formatCompactToken(contextSegments.cachedTokens)}
                percent={contextSegments.cachedPercent}
              />
              <DistributionLegend
                color={freshContextColor}
                label={t('conversation.sessionMetrics.contextFresh')}
                value={formatCompactToken(contextSegments.freshTokens)}
                percent={contextSegments.freshPercent}
              />
              <DistributionLegend
                color={remainingContextColor}
                label={t('conversation.sessionMetrics.contextRemaining')}
                value={formatCompactToken(contextSegments.remainingTokens)}
                percent={contextSegments.remainingPercent}
              />
            </div>
          </>
        )}
      </section>

      <section className='mt-12px'>
        <div className='mb-7px text-12px font-600 leading-18px'>{t('conversation.sessionMetrics.tokensTitle')}</div>
        <div className='grid grid-cols-2 gap-8px'>
          <MetricTile
            label={t('conversation.sessionMetrics.totalTokens')}
            value={formatCompactToken(usage.total_tokens)}
            caption={formatFullNumber(usage.total_tokens)}
            icon={<ChartHistogram theme='outline' size='14' />}
          />
          <MetricTile
            label={t('conversation.sessionMetrics.inputTokens')}
            value={formatCompactToken(usage.input_tokens)}
            caption={formatFullNumber(usage.input_tokens)}
          />
          <MetricTile
            label={t('conversation.sessionMetrics.outputTokens')}
            value={formatCompactToken(usage.output_tokens)}
            caption={formatFullNumber(usage.output_tokens)}
          />
          <MetricTile
            label={t('conversation.sessionMetrics.cacheHitRate')}
            value={formatPercent(cachePercent)}
            caption={t('conversation.sessionMetrics.cacheHitCaption')}
            icon={<Lightning theme='outline' size='14' />}
          />
        </div>
      </section>

      <section className='mt-12px rounded-8px border border-solid border-[var(--color-border-2)] bg-fill-1 p-10px'>
        <div className='mb-7px text-12px font-600 leading-18px'>{t('conversation.sessionMetrics.cacheTitle')}</div>
        <div className='grid grid-cols-2 gap-8px'>
          <div>
            <div className='text-11px text-t-tertiary leading-16px'>
              {t('conversation.sessionMetrics.cacheReadTokens')}
            </div>
            <div className='mt-2px text-15px font-600 tabular-nums'>{formatFullNumber(usage.cache_read_tokens)}</div>
          </div>
          <div>
            <div className='text-11px text-t-tertiary leading-16px'>
              {t('conversation.sessionMetrics.cacheWriteTokens')}
            </div>
            <div className='mt-2px text-15px font-600 tabular-nums'>
              {formatFullNumber(usage.cache_creation_tokens)}
            </div>
          </div>
        </div>
      </section>
    </div>
  );
};

export default NomiSessionMetricsPanel;
