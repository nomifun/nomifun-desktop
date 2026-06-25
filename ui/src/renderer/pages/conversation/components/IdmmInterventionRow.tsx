/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import type { IIdmmIntervention } from '@/common/adapter/ipcBridge';

/** epoch ms → 本地 `MM-DD HH:mm:ss`(决策记录时间列,无需国际化日期格式)。 */
export const formatLogTime = (at: number): string => {
  const d = new Date(at);
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
};

/** 值守徽标配色:fault=warning(故障值守)、decision=primary(决策值守),其余中性。 */
const watchBadgeStyle = (watch: string): React.CSSProperties => {
  const color =
    watch === 'fault' ? 'var(--warning-6)' : watch === 'decision' ? 'var(--primary-6)' : 'var(--gray-6)';
  return { color: `rgb(${color})`, backgroundColor: `rgba(${color}, 0.12)` };
};

/** 结果徽标配色:applied/resolved=success,failed/halted=danger,其余中性。 */
const outcomeBadgeStyle = (outcome: string): React.CSSProperties => {
  const ok = outcome === 'applied' || outcome === 'resolved';
  const bad = outcome === 'failed' || outcome === 'halted';
  const color = ok ? 'var(--success-6)' : bad ? 'var(--danger-6)' : 'var(--gray-6)';
  return { color: `rgb(${color})`, backgroundColor: `rgba(${color}, 0.12)` };
};

/**
 * 单条 IDMM 决策记录(「思路」审计行)的展示。被会话内时间线
 * (`IdmmControl`)与全局决策活动总览(`IdmmActivityContent`)共用,
 * 保证两处行布局完全一致。纯展示组件,不读后端。
 */
const IdmmInterventionRow: React.FC<{ rec: IIdmmIntervention }> = ({ rec }) => {
  const { t } = useTranslation();
  return (
    <div className='flex flex-col gap-2px rounded-6px bg-[rgb(var(--gray-2))] px-8px py-6px'>
      <div className='flex items-center gap-6px flex-wrap'>
        <span className='text-t-tertiary text-10px tabular-nums'>{formatLogTime(rec.at)}</span>
        <span
          className='inline-flex items-center rounded-4px px-4px text-10px leading-16px'
          style={watchBadgeStyle(rec.watch)}
        >
          {t(`idmm.log.watch.${rec.watch}`, rec.watch)}
        </span>
        <span className='text-t-tertiary text-10px'>
          {t('idmm.log.signal')}: {rec.stall_class}
        </span>
        <span className='text-t-tertiary text-10px'>
          {t('idmm.log.tier')}: {t(`idmm.log.tierValue.${rec.tier_used}`, rec.tier_used)}
          {rec.bypass_model ? ` · ${rec.bypass_model}` : ''}
        </span>
      </div>
      <div className='flex items-center gap-6px flex-wrap'>
        {rec.category ? (
          <span className='text-t-tertiary text-10px'>
            {t('idmm.log.category')}: {t(`idmm.log.categoryValue.${rec.category}`, rec.category)}
          </span>
        ) : null}
        <span className='text-t-secondary text-11px font-600'>
          {t(`idmm.log.actionValue.${rec.action}`, rec.action)}
        </span>
        <span
          className='inline-flex items-center rounded-4px px-4px text-10px leading-16px'
          style={outcomeBadgeStyle(rec.outcome)}
        >
          {t(`idmm.log.outcomeValue.${rec.outcome}`, rec.outcome)}
        </span>
        {typeof rec.confidence === 'number' ? (
          <span className='text-t-tertiary text-10px tabular-nums'>
            {t('idmm.log.confidence')}: {Math.round(rec.confidence * 100)}%
          </span>
        ) : null}
      </div>
      {rec.detail ? (
        <div className='text-t-secondary text-11px leading-15px break-words'>
          {t('idmm.log.detail')}: {rec.detail}
        </div>
      ) : null}
      {rec.reason ? (
        <div className='text-t-tertiary text-11px leading-15px break-words'>
          {t('idmm.log.reason')}: {rec.reason}
        </div>
      ) : null}
    </div>
  );
};

export default IdmmInterventionRow;
