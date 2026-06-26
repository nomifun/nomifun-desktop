/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { Left, Pause } from '@icon-park/react';
import { Popconfirm } from '@arco-design/web-react';
import type { TRun } from '@/common/types/orchestrator/orchestratorTypes';

/** Run status → theme-var color + i18n key suffix (mirrors RunHistory). */
const RUN_STATUS_META: Record<string, { color: string; key: string }> = {
  planning: { color: 'var(--warning)', key: 'planning' },
  running: { color: 'rgb(var(--primary-6))', key: 'running' },
  completed: { color: 'var(--success)', key: 'completed' },
  failed: { color: 'var(--danger)', key: 'failed' },
  cancelled: { color: 'var(--bg-6)', key: 'cancelled' },
};

interface RunDetailHeaderProps {
  run: TRun;
  /** done / total task counts for the aggregate progress pill. */
  done: number;
  total: number;
  onBack: () => void;
  /** Cancel the run (already wired to the REST call + toast by the parent). */
  onCancel: () => void;
  cancelling: boolean;
}

/**
 * RunDetailHeader — the top bar of the run-detail (DAG) view: a back button,
 * the run goal + status badge, an aggregate done/total progress pill, and a
 * confirm-guarded cancel action. Theme variables only; the cancel button is
 * hidden once the run reaches a terminal state.
 */
const RunDetailHeader: React.FC<RunDetailHeaderProps> = ({ run, done, total, onBack, onCancel, cancelling }) => {
  const { t } = useTranslation();
  const meta = RUN_STATUS_META[run.status];
  const dotColor = meta?.color ?? 'var(--bg-6)';
  const statusLabel = t(`orchestrator.run.status.${meta?.key ?? 'unknown'}`);
  const goalText = run.goal.trim() || t('orchestrator.run.untitledGoal');
  const isTerminal = run.status === 'completed' || run.status === 'failed' || run.status === 'cancelled';
  const pct = total > 0 ? Math.round((done / total) * 100) : 0;

  return (
    <div className='flex shrink-0 items-center gap-12px border-b border-b-base bg-1 px-16px py-12px'>
      {/* Back */}
      <div
        role='button'
        tabIndex={0}
        aria-label={t('orchestrator.run.detail.back')}
        onClick={onBack}
        onKeyDown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            onBack();
          }
        }}
        className='flex size-30px shrink-0 cursor-pointer items-center justify-center rd-8px text-t-secondary transition-colors hover:bg-fill-2 hover:text-t-primary'
      >
        <Left theme='outline' size='18' strokeWidth={3} />
      </div>

      {/* Goal + status */}
      <div className='min-w-0 flex-1'>
        <div className='truncate text-15px font-600 leading-tight text-t-primary'>{goalText}</div>
        <div className='mt-3px flex items-center gap-6px text-12px text-t-tertiary'>
          <span className='inline-flex items-center gap-4px shrink-0'>
            <span className='size-7px shrink-0 rd-full' style={{ backgroundColor: dotColor }} />
            <span style={{ color: dotColor }}>{statusLabel}</span>
          </span>
        </div>
      </div>

      {/* Aggregate progress pill */}
      <div className='hidden shrink-0 items-center gap-8px sm:flex'>
        <div className='h-6px w-100px overflow-hidden rd-full' style={{ background: 'var(--bg-3)' }}>
          <div
            className='h-full rd-full transition-all duration-300'
            style={{ width: `${pct}%`, background: 'rgb(var(--primary-6))' }}
          />
        </div>
        <span className='text-12px font-500 tabular-nums text-t-secondary'>
          {done}/{total}
        </span>
      </div>

      {/* Cancel (confirm-guarded), only while active */}
      {!isTerminal && (
        <Popconfirm
          focusLock
          title={t('orchestrator.run.detail.cancelConfirm')}
          okText={t('orchestrator.run.detail.cancelConfirmOk')}
          cancelText={t('orchestrator.run.detail.cancelConfirmCancel')}
          onOk={onCancel}
        >
          <div
            role='button'
            tabIndex={0}
            aria-label={t('orchestrator.run.detail.cancel')}
            aria-disabled={cancelling}
            className='flex h-30px shrink-0 cursor-pointer items-center gap-5px rd-8px border border-b-base px-10px text-12px font-500 text-t-secondary transition-colors hover:border-danger hover:text-danger'
            style={cancelling ? { opacity: 0.6, pointerEvents: 'none' } : undefined}
          >
            <Pause theme='outline' size='14' strokeWidth={3} />
            <span>{t('orchestrator.run.detail.cancel')}</span>
          </div>
        </Popconfirm>
      )}
    </div>
  );
};

export default RunDetailHeader;
