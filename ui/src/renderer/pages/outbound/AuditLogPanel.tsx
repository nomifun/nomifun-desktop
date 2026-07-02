/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { TFunction } from 'i18next';
import { Empty, Spin } from '@arco-design/web-react';
import { Comment, History, Power, Refresh } from '@icon-park/react';
import { ipcBridge } from '@/common';
import type { CompanionAuditSurface, ICompanionAuditEntry } from '@/common/adapter/ipcBridge';

interface Props {
  companionId: string;
}

/** Relative "N 分钟前" formatter; falls back to a locale date past a week. */
const formatRelative = (t: TFunction, at: number): string => {
  const diff = Date.now() - at;
  const MIN = 60_000;
  const HOUR = 3_600_000;
  const DAY = 86_400_000;
  if (diff < 0 || diff < MIN) return t('outbound.audit.justNow', { defaultValue: '刚刚' });
  if (diff < HOUR) return t('outbound.audit.minutesAgo', { defaultValue: '{{n}} 分钟前', n: Math.floor(diff / MIN) });
  if (diff < DAY) return t('outbound.audit.hoursAgo', { defaultValue: '{{n}} 小时前', n: Math.floor(diff / HOUR) });
  if (diff < 7 * DAY) return t('outbound.audit.daysAgo', { defaultValue: '{{n}} 天前', n: Math.floor(diff / DAY) });
  return new Date(at).toLocaleDateString();
};

const surfaceLabel = (t: TFunction, surface: CompanionAuditSurface, platform: string | null): string => {
  if (surface === 'channel') return platform || t('outbound.audit.surfaceChannel', { defaultValue: '社交渠道' });
  if (surface === 'remote') return t('outbound.audit.surfaceRemote', { defaultValue: '远程' });
  return t('outbound.audit.surfaceDesktop', { defaultValue: '桌面' });
};

/**
 * 审计日志面板 —— 倒序展示外呼员工的对外活动（对话回合 + 对外服务开关变更）。
 * 后端接口未就绪或 404 时优雅降级为空态「暂无记录」。
 */
const AuditLogPanel: React.FC<Props> = ({ companionId }) => {
  const { t } = useTranslation();
  const [entries, setEntries] = useState<ICompanionAuditEntry[]>([]);
  const [loading, setLoading] = useState(true);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const res = await ipcBridge.companion.getCompanionAudit.invoke({ companion_id: companionId, limit: 50 });
      setEntries(res?.entries ?? []);
    } catch {
      // Endpoint may 404 while the backend is still shipping — degrade to empty.
      setEntries([]);
    } finally {
      setLoading(false);
    }
  }, [companionId]);

  useEffect(() => {
    void load();
  }, [load]);

  return (
    <div className='flex flex-col gap-10px'>
      <div className='flex items-center justify-between'>
        <span className='inline-flex items-center gap-6px text-13px font-600 text-t-secondary'>
          <History theme='outline' size='15' fill='currentColor' className='block' style={{ lineHeight: 0 }} />
          {t('outbound.audit.title', { defaultValue: '审计日志' })}
        </span>
        <span
          role='button'
          tabIndex={0}
          title={t('common.refresh', { defaultValue: '刷新' })}
          onClick={() => void load()}
          onKeyDown={(e) => {
            if (e.key === 'Enter' || e.key === ' ') {
              e.preventDefault();
              void load();
            }
          }}
          className='flex items-center justify-center w-26px h-26px rd-7px text-t-tertiary cursor-pointer hover:bg-fill-2 hover:text-t-primary transition-colors'
        >
          <Refresh theme='outline' size='14' fill='currentColor' className='block' style={{ lineHeight: 0 }} />
        </span>
      </div>

      {loading ? (
        <div className='flex justify-center py-32px'>
          <Spin />
        </div>
      ) : entries.length === 0 ? (
        <div className='flex justify-center py-28px'>
          <Empty description={t('outbound.audit.empty', { defaultValue: '暂无记录' })} />
        </div>
      ) : (
        <div className='flex flex-col gap-6px'>
          {entries.map((e) => {
            const isExposure = e.kind === 'exposure_change';
            return (
              <div
                key={e.id}
                className='flex items-start gap-10px rd-10px border border-solid border-[var(--color-border-2)] bg-[var(--color-bg-1)] px-11px py-9px'
              >
                <span
                  className={[
                    'mt-1px flex shrink-0 items-center justify-center w-24px h-24px rd-7px',
                    isExposure
                      ? 'text-[rgb(var(--warning-6))] bg-[rgba(var(--warning-6),0.12)]'
                      : 'text-[rgb(var(--primary-6))] bg-[rgba(var(--primary-6),0.10)]',
                  ].join(' ')}
                >
                  {isExposure ? (
                    <Power theme='outline' size='13' fill='currentColor' className='block' style={{ lineHeight: 0 }} />
                  ) : (
                    <Comment theme='outline' size='13' fill='currentColor' className='block' style={{ lineHeight: 0 }} />
                  )}
                </span>
                <div className='min-w-0 flex-1'>
                  <div className='flex items-center gap-6px flex-wrap'>
                    <span className='inline-flex items-center rd-full px-7px py-1px text-10px font-600 leading-none text-t-secondary bg-fill-2 border border-solid border-[var(--color-border-2)]'>
                      {surfaceLabel(t, e.surface, e.channel_platform)}
                    </span>
                    <span className='text-11px text-t-tertiary'>{formatRelative(t, e.at)}</span>
                  </div>
                  <div className='mt-3px text-13px leading-18px text-t-primary break-words'>{e.detail}</div>
                </div>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
};

export default AuditLogPanel;
