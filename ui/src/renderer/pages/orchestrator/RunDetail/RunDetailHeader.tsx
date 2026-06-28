/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { CheckOne, Left, Pause, PauseOne, PlayOne, Refresh, Time } from '@icon-park/react';
import { Popconfirm } from '@arco-design/web-react';
import type { TRun } from '@/common/types/orchestrator/orchestratorTypes';
import { formatDuration } from './runFormat';
import { taskStatusMeta } from './nodes/TaskNode';

/** Run status → theme-var color + i18n key suffix (mirrors RunHistory). */
const RUN_STATUS_META: Record<string, { color: string; key: string }> = {
  planning: { color: 'var(--warning)', key: 'planning' },
  running: { color: 'rgb(var(--primary-6))', key: 'running' },
  paused: { color: 'var(--warning)', key: 'paused' },
  awaiting_plan_approval: { color: 'rgb(var(--primary-6))', key: 'awaiting_plan_approval' },
  completed: { color: 'var(--success)', key: 'completed' },
  failed: { color: 'var(--danger)', key: 'failed' },
  cancelled: { color: 'var(--bg-6)', key: 'cancelled' },
};

/** Order in which non-zero per-status task counts are shown in the breakdown. */
const STATUS_BREAKDOWN_ORDER = [
  'running',
  'pending',
  'blocked',
  'needs_review',
  'failed',
  'done',
  'completed',
  'skipped',
  'cancelled',
];

interface RunDetailHeaderProps {
  run: TRun;
  /** done / total task counts for the aggregate progress pill. */
  done: number;
  total: number;
  /** Per-task-status counts for the status breakdown chips. */
  byStatus: Record<string, number>;
  /** Title of the currently-running task (if any) — the "what's happening now" hint. */
  currentTitle: string | null;
  /** Summed token usage across the run's tasks (0 = hide). */
  totalTokens: number;
  /** Run elapsed time in ms (created_at → updated_at). 0 = hide. */
  elapsedMs: number;
  onBack: () => void;
  /** Cancel the run (already wired to the REST call + toast by the parent). */
  onCancel: () => void;
  /** Approve an interactive run's plan (`awaiting_plan_approval` → `running`). */
  onApprove: () => void;
  /** Pause a running run (`running` → `paused`). */
  onPause: () => void;
  /** Resume a paused run (`paused` → `running`). */
  onResume: () => void;
  /** Open the in-place re-plan editor for this run. Omitted in embedded mode. */
  onReplan?: () => void;
  /** While any control action (cancel/approve/pause/resume) is in flight. */
  busy: boolean;
  /**
   * Embedded mode — when the header is rendered inside a conversation's
   * workspace rail tab there is no master-detail to navigate back to, so the
   * back button is suppressed. Run controls are always kept.
   */
  embedded?: boolean;
}

/**
 * RunDetailHeader — the top bar of the run-detail (DAG) view: a back button,
 * the run goal + status badge, a progress cluster (status-breakdown chips +
 * elapsed + tokens + aggregate pill), the current-task hint, and the
 * status-aware control actions. Theme variables only.
 *
 * Control buttons (P3b):
 *  - `awaiting_plan_approval` → 「批准计划」(approve, primary).
 *  - `running` → 「暂停」(pause).
 *  - `paused` → 「继续」(resume).
 *  - 「重新规划」(re-plan) opens the in-place editor for any run.
 *  - the confirm-guarded 「终止 Run」(cancel) stays for any non-terminal run.
 */
const RunDetailHeader: React.FC<RunDetailHeaderProps> = ({
  run,
  done,
  total,
  byStatus,
  currentTitle,
  totalTokens,
  elapsedMs,
  onBack,
  onCancel,
  onApprove,
  onPause,
  onResume,
  onReplan,
  busy,
  embedded,
}) => {
  const { t } = useTranslation();
  const meta = RUN_STATUS_META[run.status];
  const dotColor = meta?.color ?? 'var(--bg-6)';
  const statusLabel = t(`orchestrator.run.status.${meta?.key ?? 'unknown'}`);
  const goalText = run.goal.trim() || t('orchestrator.run.untitledGoal');
  const isTerminal = run.status === 'completed' || run.status === 'failed' || run.status === 'cancelled';
  const pct = total > 0 ? Math.round((done / total) * 100) : 0;
  const busyStyle = busy ? { opacity: 0.6, pointerEvents: 'none' as const } : undefined;

  // Non-zero per-status counts, in display order, with task-status colors.
  const breakdown = STATUS_BREAKDOWN_ORDER.filter((s) => (byStatus[s] ?? 0) > 0).map((s) => ({
    status: s,
    count: byStatus[s] ?? 0,
    color: taskStatusMeta(s).color,
    label: t(`orchestrator.run.task.status.${s}`, { defaultValue: s }),
  }));

  /** Fire a control callback from keyboard activation (Enter / Space). */
  const onKeyActivate = (fn: () => void) => (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      fn();
    }
  };

  return (
    <div className='flex shrink-0 items-center gap-12px border-b border-b-base bg-1 px-16px py-12px'>
      {/* Back — suppressed in embedded (rail) mode, which has no detail to return to. */}
      {!embedded && (
        <div
          role='button'
          tabIndex={0}
          aria-label={t('orchestrator.run.detail.back')}
          onClick={onBack}
          onKeyDown={onKeyActivate(onBack)}
          className='flex size-30px shrink-0 cursor-pointer items-center justify-center rd-8px text-t-secondary transition-colors hover:bg-fill-2 hover:text-t-primary'
        >
          <Left theme='outline' size='18' strokeWidth={3} />
        </div>
      )}

      {/* Goal + status + pacing line */}
      <div className='min-w-0 flex-1'>
        <div className='truncate text-15px font-600 leading-tight text-t-primary'>{goalText}</div>
        <div className='mt-3px flex items-center gap-10px text-12px text-t-tertiary'>
          <span className='inline-flex shrink-0 items-center gap-4px'>
            <span className='size-7px shrink-0 rd-full' style={{ backgroundColor: dotColor }} />
            <span style={{ color: dotColor }}>{statusLabel}</span>
          </span>
          {elapsedMs > 0 && (
            <span className='inline-flex shrink-0 items-center gap-3px tabular-nums'>
              <Time theme='outline' size='12' strokeWidth={3} />
              {formatDuration(elapsedMs)}
            </span>
          )}
          {totalTokens > 0 && (
            <span className='shrink-0 tabular-nums'>
              {totalTokens.toLocaleString()} {t('orchestrator.run.progress.tokens')}
            </span>
          )}
          {currentTitle && (
            <span className='inline-flex min-w-0 items-center gap-4px'>
              <span className='shrink-0'>·</span>
              <span className='shrink-0 text-primary-6'>{t('orchestrator.run.progress.current')}</span>
              <span className='truncate' title={currentTitle}>
                {currentTitle}
              </span>
            </span>
          )}
        </div>
      </div>

      {/* Status-breakdown chips */}
      {breakdown.length > 0 && (
        <div className='hidden shrink-0 items-center gap-8px md:flex'>
          {breakdown.map((b) => (
            <span
              key={b.status}
              className='inline-flex items-center gap-4px text-12px tabular-nums text-t-secondary'
              title={b.label}
            >
              <span className='size-7px shrink-0 rd-full' style={{ background: b.color }} />
              {b.count}
            </span>
          ))}
        </div>
      )}

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

      {/* Re-plan (in-place re-design) — available for any run (standalone page only). */}
      {onReplan && (
        <div
          role='button'
          tabIndex={0}
          aria-label={t('orchestrator.run.detail.replan')}
          onClick={onReplan}
          onKeyDown={onKeyActivate(onReplan)}
          className='flex h-30px shrink-0 cursor-pointer items-center gap-5px rd-8px border border-b-base px-10px text-12px font-500 text-t-secondary transition-colors hover:border-primary-6 hover:text-primary-6'
        >
          <Refresh theme='outline' size='14' strokeWidth={3} />
          <span>{t('orchestrator.run.detail.replan')}</span>
        </div>
      )}

      {/* Approve plan (interactive run, primary) */}
      {run.status === 'awaiting_plan_approval' && (
        <div
          role='button'
          tabIndex={0}
          aria-label={t('orchestrator.run.detail.approvePlan')}
          aria-disabled={busy}
          onClick={busy ? undefined : onApprove}
          onKeyDown={onKeyActivate(onApprove)}
          className='flex h-30px shrink-0 cursor-pointer items-center gap-5px rd-8px px-10px text-12px font-500 text-white transition-opacity hover:opacity-90'
          style={{ background: 'rgb(var(--primary-6))', ...busyStyle }}
        >
          <CheckOne theme='outline' size='14' strokeWidth={3} />
          <span>{t('orchestrator.run.detail.approvePlan')}</span>
        </div>
      )}

      {/* Pause (running) */}
      {run.status === 'running' && (
        <div
          role='button'
          tabIndex={0}
          aria-label={t('orchestrator.run.detail.pause')}
          aria-disabled={busy}
          onClick={busy ? undefined : onPause}
          onKeyDown={onKeyActivate(onPause)}
          className='flex h-30px shrink-0 cursor-pointer items-center gap-5px rd-8px border border-b-base px-10px text-12px font-500 text-t-secondary transition-colors hover:border-primary-6 hover:text-primary-6'
          style={busyStyle}
        >
          <PauseOne theme='outline' size='14' strokeWidth={3} />
          <span>{t('orchestrator.run.detail.pause')}</span>
        </div>
      )}

      {/* Resume (paused) */}
      {run.status === 'paused' && (
        <div
          role='button'
          tabIndex={0}
          aria-label={t('orchestrator.run.detail.resume')}
          aria-disabled={busy}
          onClick={busy ? undefined : onResume}
          onKeyDown={onKeyActivate(onResume)}
          className='flex h-30px shrink-0 cursor-pointer items-center gap-5px rd-8px border border-b-base px-10px text-12px font-500 text-t-secondary transition-colors hover:border-primary-6 hover:text-primary-6'
          style={busyStyle}
        >
          <PlayOne theme='outline' size='14' strokeWidth={3} />
          <span>{t('orchestrator.run.detail.resume')}</span>
        </div>
      )}

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
            aria-disabled={busy}
            className='flex h-30px shrink-0 cursor-pointer items-center gap-5px rd-8px border border-b-base px-10px text-12px font-500 text-t-secondary transition-colors hover:border-danger hover:text-danger'
            style={busyStyle}
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
