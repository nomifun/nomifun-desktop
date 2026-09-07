/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Computer-history settings page: master toggle, recorder status (state /
 * macOS permission / storage), retention note, a destructive purge behind an
 * explicit confirmation, and the recent-activity view (per-app rollup +
 * segment list over a selectable time window).
 *
 * Data comes from the backend `computer_history_*` surface via
 * `ipcBridge.computerHistory`; the master switch persists the
 * `feature.computerHistory` client preference.
 */

import { ipcBridge } from '@/common';
import type {
  ComputerHistoryWindow,
  IComputerHistoryAppUsageRow,
  IComputerHistorySegment,
  IComputerHistoryStatus,
} from '@/common/adapter/ipcBridge';
import { configService } from '@/common/config/configService';
import { formatBytes } from '@renderer/utils/file/formatBytes';
import { Attention, HardDisk, History, Info, Refresh, Time } from '@icon-park/react';
import { Button, Empty, Message, Modal, Spin, Switch } from '@arco-design/web-react';
import classNames from 'classnames';
import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import SettingsPageWrapper from '../components/SettingsPageWrapper';
import {
  COMPUTER_HISTORY_WINDOWS,
  computerHistoryWindowRange,
  formatComputerHistoryRollupDuration,
  formatComputerHistorySegmentTime,
} from './windowRange';
import './computerHistorySettings.css';

const SEGMENT_LIST_LIMIT = 50;
const APP_USAGE_LIMIT = 6;
const DEFAULT_RETENTION_DAYS = 30;

const stateColorClass = (state: IComputerHistoryStatus['state']): string =>
  state === 'running' ? 'text-#16a34a' : state === 'paused' ? 'text-#f59e0b' : 'text-t-tertiary';

const permissionColorClass = (permission: IComputerHistoryStatus['permission']): string =>
  permission === 'granted' ? 'text-#16a34a' : permission === 'denied' ? 'text-#ef4444' : 'text-t-tertiary';

/** One labelled value row inside a status/description card. */
const StatusRow: React.FC<{ label: string; children: React.ReactNode }> = ({ label, children }) => (
  <div className='flex items-center justify-between gap-24px py-8px'>
    <span className='shrink-0 text-13px text-t-secondary'>{label}</span>
    <span className='min-w-0 truncate text-right text-13px text-t-primary'>{children}</span>
  </div>
);

const ComputerHistorySettings: React.FC = () => {
  const { t } = useTranslation();
  const [enabled, setEnabled] = useState(false);
  const [status, setStatus] = useState<IComputerHistoryStatus | null>(null);
  const [statusLoading, setStatusLoading] = useState(true);
  const [window, setWindow] = useState<ComputerHistoryWindow>('today');
  const [usage, setUsage] = useState<IComputerHistoryAppUsageRow[]>([]);
  const [segments, setSegments] = useState<IComputerHistorySegment[]>([]);
  const [timelineLoading, setTimelineLoading] = useState(true);

  useEffect(() => {
    setEnabled(configService.get('feature.computerHistory') ?? false);
  }, []);

  const refreshStatus = useCallback(() => {
    setStatusLoading(true);
    ipcBridge.computerHistory.status
      .invoke()
      .then(setStatus)
      .catch((error) => {
        console.warn('[computerHistory] status unavailable:', error);
        setStatus(null);
      })
      .finally(() => setStatusLoading(false));
  }, []);

  const loadTimeline = useCallback((win: ComputerHistoryWindow) => {
    setTimelineLoading(true);
    const range = computerHistoryWindowRange(win);
    // Window bounds come from the shared pure module; the rollup and the raw
    // segment list read the same bracket so the two views cannot disagree.
    void Promise.all([
      ipcBridge.computerHistory.appUsage.invoke({ ...range, limit: APP_USAGE_LIMIT }).catch(() => []),
      ipcBridge.computerHistory.list
        .invoke({ ...range, limit: SEGMENT_LIST_LIMIT })
        .catch((error): IComputerHistorySegment[] => {
          console.warn('[computerHistory] list unavailable:', error);
          return [];
        }),
    ])
      .then(([usageRows, segmentRows]) => {
        setUsage(usageRows);
        setSegments(segmentRows);
      })
      .finally(() => setTimelineLoading(false));
  }, []);

  useEffect(() => {
    refreshStatus();
    // Re-probe when the user returns from System Settings — the macOS TCC
    // grant state is the whole reason to revisit this panel.
    const onFocus = () => refreshStatus();
    globalThis.addEventListener?.('focus', onFocus);
    return () => globalThis.removeEventListener?.('focus', onFocus);
  }, [refreshStatus]);

  useEffect(() => {
    loadTimeline(window);
  }, [loadTimeline, window]);

  const handleEnabledChange = useCallback((checked: boolean) => {
    setEnabled(checked);
    configService
      .set('feature.computerHistory', checked)
      .then(() => ipcBridge.computerHistory.setEnabled.invoke({ enabled: checked }))
      .then(() => {
        refreshStatus();
      })
      .catch((error) => {
        console.error('[computerHistory] set enabled failed:', error);
        setEnabled(!checked);
        configService.setLocal('feature.computerHistory', !checked);
        Message.error(t('computerHistory.errorSaveFailed'));
      });
  }, [refreshStatus, t]);

  const handlePurge = useCallback(() => {
    const count = status?.storage.segments ?? 0;
    Modal.confirm({
      title: t('computerHistory.purgeConfirmTitle'),
      content: t('computerHistory.purgeConfirmBody', { count }),
      okButtonProps: { status: 'danger' },
      okText: t('computerHistory.purgeConfirmAction'),
      cancelText: t('computerHistory.purgeCancel'),
      onOk: async () => {
        try {
          await ipcBridge.computerHistory.purge.invoke({});
          Message.success(t('computerHistory.storagePurge'));
          refreshStatus();
          loadTimeline(window);
        } catch (error) {
          console.error('[computerHistory] purge failed:', error);
          Message.error(t('computerHistory.errorPurgeFailed'));
        }
      },
    });
  }, [loadTimeline, refreshStatus, status?.storage.segments, t, window]);

  const timelineEmpty = !timelineLoading && segments.length === 0;
  const storagePath = status?.storage.path;

  const topApps = useMemo(() => {
    const max = usage.reduce((acc, row) => Math.max(acc, row.total_ms), 0) || 1;
    return usage.map((row) => ({ ...row, ratio: row.total_ms / max }));
  }, [usage]);

  return (
    <SettingsPageWrapper contentClassName='max-w-800px'>
      <header className='mb-18px'>
        <h1 className='m-0 text-20px font-600 leading-28px text-t-primary'>{t('computerHistory.title')}</h1>
        <p className='m-0 mt-4px text-12px leading-18px text-t-secondary'>{t('computerHistory.description')}</p>
      </header>

      {/* Master switch */}
      <section className='mb-16px flex items-center justify-between gap-24px rd-12px border border-solid border-[var(--color-border-2)] bg-fill-2 px-16px py-14px'>
        <div className='min-w-0'>
          <div className='text-14px font-500 text-t-primary'>{t('computerHistory.enableLabel')}</div>
          <div className='mt-4px text-12px leading-18px text-t-tertiary'>{t('computerHistory.enableHint')}</div>
        </div>
        <Switch checked={enabled} onChange={handleEnabledChange} />
      </section>

      {/* Status card */}
      <section className='mb-16px rd-12px border border-solid border-[var(--color-border-2)] bg-fill-2 px-16px py-14px'>
        <div className='mb-6px flex items-center justify-between'>
          <div className='flex items-center gap-6px text-13px font-600 text-t-secondary'>
            <History theme='outline' size='14' strokeWidth={3} className='text-t-secondary' />
            {t('computerHistory.statusTitle')}
          </div>
          <Button
            size='mini'
            icon={<Refresh theme='outline' size='12' strokeWidth={3} />}
            onClick={refreshStatus}
          >
            {t('computerHistory.statusRefresh')}
          </Button>
        </div>
        {statusLoading && !status ? (
          <div className='flex min-h-64px items-center justify-center'>
            <Spin />
          </div>
        ) : status ? (
          <div className='w-full'>
            <StatusRow label={t('computerHistory.statusStateLabel')}>
              <span className={classNames('font-600', stateColorClass(status.state))}>
                {t(`computerHistory.statusState${status.state.charAt(0).toUpperCase()}${status.state.slice(1)}`)}
              </span>
            </StatusRow>
            <StatusRow label={t('computerHistory.statusPermissionLabel')}>
              <span className={classNames('font-600', permissionColorClass(status.permission))}>
                {t(
                  `computerHistory.statusPermission${status.permission.charAt(0).toUpperCase()}${status.permission.slice(1)}`
                )}
              </span>
            </StatusRow>
            <StatusRow label={t('computerHistory.statusStorageLabel')}>
              {t('computerHistory.statusStorageUsage', {
                segments: status.storage.segments,
                bytes: formatBytes(status.storage.approx_bytes),
              })}
            </StatusRow>
            <StatusRow label={t('computerHistory.statusStoragePath')}>
              <span className='font-mono text-12px text-t-secondary'>{storagePath}</span>
            </StatusRow>
            {status.chat_analytics != null && (
              <StatusRow label={t('computerHistory.statusChatAnalytics')}>
                <span
                  className={classNames(
                    'font-600',
                    status.chat_analytics.available ? 'text-#16a34a' : 'text-t-tertiary'
                  )}
                >
                  {status.chat_analytics.available
                    ? t('computerHistory.statusChatAnalyticsAvailable')
                    : t('computerHistory.statusChatAnalyticsUnavailable')}
                </span>
              </StatusRow>
            )}
          </div>
        ) : (
          <div className='flex items-center gap-6px py-8px text-13px text-t-tertiary'>
            <Info theme='outline' size='14' strokeWidth={3} />
            {t('computerHistory.errorLoadFailed')}
          </div>
        )}
      </section>

      {/* Permission guidance — only when capture is on but macOS has not granted */}
      {enabled && status?.permission === 'denied' && (
        <section className='mb-16px flex gap-10px rd-12px border border-solid border-[var(--color-border-2)] bg-fill-2 px-16px py-14px'>
          <Attention theme='outline' size='18' strokeWidth={3} className='mt-2px shrink-0 text-#f59e0b' />
          <div className='min-w-0 flex-1'>
            <div className='text-13px font-600 text-t-primary'>{t('computerHistory.permissionTitle')}</div>
            <p className='mb-6px mt-4px text-12px leading-18px text-t-secondary'>
              {t('computerHistory.permissionNeeded')}
            </p>
            <p className='mb-8px mt-0 text-12px leading-18px text-t-tertiary'>
              {t('computerHistory.permissionHowTo')}
            </p>
            <Button
              size='small'
              onClick={() => {
                void ipcBridge.computerPermissions.openSettings
                  .invoke({ kind: 'accessibility' })
                  .catch(() => {});
              }}
            >
              {t('computerHistory.permissionOpenSettings')}
            </Button>
          </div>
        </section>
      )}

      {/* Retention + destructive purge */}
      <section className='mb-16px flex items-center justify-between gap-24px rd-12px border border-solid border-[var(--color-border-2)] bg-fill-2 px-16px py-14px'>
        <div className='flex min-w-0 items-center gap-8px'>
          <HardDisk theme='outline' size='16' strokeWidth={3} className='shrink-0 text-t-secondary' />
          <div className='min-w-0'>
            <div className='text-13px font-500 text-t-primary'>{t('computerHistory.retentionTitle')}</div>
            <div className='mt-2px text-12px leading-18px text-t-tertiary'>
              {t('computerHistory.retentionHint', { days: DEFAULT_RETENTION_DAYS })}
            </div>
          </div>
        </div>
        <Button status='danger' size='small' onClick={handlePurge}>
          {t('computerHistory.storagePurge')}
        </Button>
      </section>

      {/* Activity timeline */}
      <section className='rd-12px border border-solid border-[var(--color-border-2)] bg-fill-2 px-16px py-14px'>
        <div className='mb-10px flex flex-wrap items-center justify-between gap-8px'>
          <div className='flex items-center gap-6px text-13px font-600 text-t-secondary'>
            <Time theme='outline' size='14' strokeWidth={3} className='text-t-secondary' />
            {t('computerHistory.timelineTitle')}
          </div>
          <div className='flex items-center gap-4px'>
            {COMPUTER_HISTORY_WINDOWS.map((win) => {
              const selected = window === win;
              return (
                <button
                  key={win}
                  type='button'
                  aria-pressed={selected}
                  className={classNames(
                    'appearance-none border-0 bg-transparent px-9px py-4px text-13px leading-18px outline-none transition-colors',
                    selected ? 'rd-8px bg-fill-3 font-500 text-t-primary' : 'text-t-secondary hover:text-t-primary'
                  )}
                  onClick={() => setWindow(win)}
                >
                  {t(`computerHistory.window${win.charAt(0).toUpperCase()}${win.slice(1)}`)}
                </button>
              );
            })}
          </div>
        </div>

        {timelineLoading ? (
          <div className='flex min-h-160px items-center justify-center'>
            <Spin />
          </div>
        ) : timelineEmpty ? (
          <div className='flex min-h-160px items-center justify-center [&_.arco-empty-image]:!text-56px'>
            <Empty description={t('computerHistory.timelineEmpty')} />
          </div>
        ) : (
          <div className='flex flex-col gap-14px'>
            {/* Top apps rollup */}
            {topApps.length > 0 && (
              <div>
                <div className='mb-6px text-12px font-500 text-t-tertiary'>{t('computerHistory.topAppsTitle')}</div>
                <div className='flex flex-col gap-6px'>
                  {topApps.map((row) => {
                    const duration = formatComputerHistoryRollupDuration(row.total_ms);
                    return (
                      <div key={row.app_name} className='flex items-center gap-10px'>
                        <span className='w-120px shrink-0 truncate text-13px text-t-primary'>{row.app_name}</span>
                        <div className='h-6px min-w-0 flex-1 overflow-hidden rd-3px bg-fill-3'>
                          <div className='h-full rd-3px bg-primary-6' style={{ width: `${Math.max(4, row.ratio * 100)}%` }} />
                        </div>
                        <span className='shrink-0 text-12px text-t-secondary'>
                          {duration.hours > 0
                            ? t('computerHistory.durationHoursMinutes', duration)
                            : t('computerHistory.durationMinutes', { count: duration.minutesOnly })}
                        </span>
                      </div>
                    );
                  })}
                </div>
              </div>
            )}

            {/* Recent segments */}
            <div className='flex flex-col'>
              <div className='ch-settings-segment-row ch-settings-segment-row--header text-12px font-500 text-t-tertiary'>
                <span>{t('computerHistory.segmentColumnApp')}</span>
                <span>{t('computerHistory.segmentColumnTitle')}</span>
                <span>{t('computerHistory.segmentColumnUrl')}</span>
                <span className='text-right'>{t('computerHistory.segmentColumnTime')}</span>
              </div>
              <div className='flex flex-col divide-y divide-x-0 divide-solid divide-[var(--color-border-2)]'>
                {segments.map((segment) => (
                  <div key={segment.event_id} className='ch-settings-segment-row text-13px'>
                    <span className='truncate text-t-primary'>{segment.app_name}</span>
                    <span className='truncate text-t-secondary'>{segment.window_title || '—'}</span>
                    <span className='truncate text-12px text-t-tertiary'>
                      {segment.browser_url ? (
                        <a className='ch-settings-segment-link' href={segment.browser_url} target='_blank' rel='noreferrer'>
                          {segment.browser_url}
                        </a>
                      ) : (
                        '—'
                      )}
                    </span>
                    <span className='text-right text-12px text-t-secondary'>
                      {formatComputerHistorySegmentTime(segment.started_at_ms, segment.ended_at_ms)}
                    </span>
                  </div>
                ))}
              </div>
            </div>
          </div>
        )}
      </section>
    </SettingsPageWrapper>
  );
};

export default ComputerHistorySettings;
