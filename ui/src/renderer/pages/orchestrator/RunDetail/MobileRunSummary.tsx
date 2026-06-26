/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { Spin } from '@arco-design/web-react';
import { Branch, Left } from '@icon-park/react';
import { taskStatusMeta } from './nodes/TaskNode';
import { useRunLive } from '../useRunLive';

/** Map a run status string to a theme-var color + i18n label key suffix. */
const RUN_STATUS_META: Record<string, { color: string; key: string }> = {
  planning: { color: 'var(--warning)', key: 'planning' },
  running: { color: 'rgb(var(--primary-6))', key: 'running' },
  completed: { color: 'var(--success)', key: 'completed' },
  failed: { color: 'var(--danger)', key: 'failed' },
  cancelled: { color: 'var(--color-text-3)', key: 'cancelled' },
};

interface MobileRunSummaryProps {
  runId: string;
  onBack: () => void;
}

/**
 * MobileRunSummary — the read-only run view shown on mobile in place of the
 * interactive DAG canvas (which is awkward on small screens). Renders the run's
 * goal, status, and a flat task list with per-task status dots; live-updated via
 * {@link useRunLive}. Tapping a task is intentionally inert here — the worker
 * transcript drawer is a desktop affordance (P1b scope boundary).
 */
const MobileRunSummary: React.FC<MobileRunSummaryProps> = ({ runId, onBack }) => {
  const { t } = useTranslation();
  const { detail, loading } = useRunLive(runId);

  const backButton = (
    <div
      role='button'
      tabIndex={0}
      onClick={onBack}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          onBack();
        }
      }}
      className='inline-flex items-center gap-4px text-13px text-t-secondary cursor-pointer select-none hover:text-t-primary'
    >
      <Left theme='outline' size='16' strokeWidth={3} />
      <span>{t('orchestrator.run.detail.back')}</span>
    </div>
  );

  if (loading && !detail) {
    return (
      <div className='w-full'>
        <div className='mb-12px'>{backButton}</div>
        <div className='py-48px flex items-center justify-center'>
          <Spin />
        </div>
      </div>
    );
  }

  if (!detail) {
    return (
      <div className='w-full'>
        <div className='mb-12px'>{backButton}</div>
        <div className='py-48px text-center text-13px text-t-tertiary'>{t('orchestrator.run.detail.loadError')}</div>
      </div>
    );
  }

  const { run, tasks } = detail;
  const runMeta = RUN_STATUS_META[run.status];
  const runDotColor = runMeta?.color ?? 'var(--color-text-3)';
  const runStatusLabel = t(`orchestrator.run.status.${runMeta?.key ?? 'unknown'}`);
  const goalText = run.goal.trim() || t('orchestrator.run.untitledGoal');

  return (
    <div className='w-full'>
      <div className='mb-12px'>{backButton}</div>

      <div className='rd-12px bg-1 px-16px py-14px'>
        <div className='text-15px font-600 text-t-primary leading-snug'>{goalText}</div>
        <div className='mt-6px text-12px flex items-center gap-6px'>
          <span className='size-7px rd-full shrink-0' style={{ backgroundColor: runDotColor }} />
          <span style={{ color: runDotColor }}>{runStatusLabel}</span>
        </div>
      </div>

      <div className='mt-16px text-13px font-600 text-t-secondary'>{t('orchestrator.run.mobile.tasksLabel')}</div>

      {tasks.length === 0 ? (
        <div className='mt-10px rd-12px bg-1 px-20px py-32px flex flex-col items-center justify-center text-center'>
          <span className='size-44px rd-12px bg-fill-2 text-t-tertiary flex items-center justify-center mb-12px'>
            <Branch theme='outline' size='22' strokeWidth={3} />
          </span>
          <div className='text-13px font-600 text-t-primary'>{t('orchestrator.run.detail.planningTitle')}</div>
          <div className='mt-6px text-12px leading-18px text-t-tertiary max-w-280px'>
            {t('orchestrator.run.mobile.noTasks')}
          </div>
        </div>
      ) : (
        <div className='mt-10px flex flex-col gap-8px'>
          {tasks.map((task) => {
            const meta = taskStatusMeta(task.status);
            const taskLabel = t(`orchestrator.run.task.status.${task.status}`, {
              defaultValue: t('orchestrator.run.status.unknown'),
            });
            return (
              <div key={task.id} className='rd-10px bg-1 px-14px py-12px flex items-center gap-10px'>
                <span className='size-8px rd-full shrink-0' style={{ backgroundColor: meta.color }} />
                <span className='min-w-0 flex-1 text-13px text-t-primary truncate'>
                  {task.title || t('orchestrator.run.detail.untitledTask')}
                </span>
                <span className='shrink-0 text-11px' style={{ color: meta.color }}>
                  {taskLabel}
                </span>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
};

export default MobileRunSummary;
