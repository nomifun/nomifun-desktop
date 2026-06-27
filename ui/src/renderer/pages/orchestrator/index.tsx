/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { Suspense, useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate, useSearchParams } from 'react-router-dom';
import { Spin } from '@arco-design/web-react';
import { Comment, Plus, Right, Workbench } from '@icon-park/react';
import type { TRun } from '@/common/types/orchestrator/orchestratorTypes';
import AppLoader from '@/renderer/components/layout/AppLoader';
import { useLayoutContext } from '@/renderer/hooks/context/LayoutContext';
import RunHistory from './RunHistory';
import NewRunComposer from './NewRunComposer';
import AgentRoster from './RunDetail/AgentRoster';
import WorkerTranscriptPanel from './RunDetail/WorkerTranscriptPanel';
import MobileRunSummary from './RunDetail/MobileRunSummary';
import type { OpenTaskPayload } from './RunDetail/DagCanvas';
import { useMyRuns } from './useOrchestratorData';
import { useRunLive } from './useRunLive';

// The DAG canvas pulls in react-flow (heavy) and is only mounted when a run is
// open, so it is split into its own chunk and loaded on demand.
const DagCanvas = React.lazy(() => import('./RunDetail/DagCanvas'));

/** Run status → theme-var color + i18n label key suffix (mirrors RunHistory). */
const STATUS_META: Record<string, { color: string; key: string }> = {
  planning: { color: 'var(--warning)', key: 'planning' },
  running: { color: 'rgb(var(--primary-6))', key: 'running' },
  completed: { color: 'var(--success)', key: 'completed' },
  failed: { color: 'var(--danger)', key: 'failed' },
  cancelled: { color: 'var(--color-text-3)', key: 'cancelled' },
  paused: { color: 'var(--warning)', key: 'paused' },
  awaiting_plan_approval: { color: 'var(--warning)', key: 'awaiting_plan_approval' },
};

const formatTime = (ms: number): string => new Date(ms).toLocaleString();

/**
 * A single run row in the left master list. Reuses RunHistory's visual language
 * (goal · status dot · timestamp), but instead of a navigation arrow it carries
 * a selected highlight and an optional "open conversation" jump.
 */
const RunListRow: React.FC<{
  run: TRun;
  selected: boolean;
  onSelect: () => void;
  onOpenConversation?: () => void;
}> = ({ run, selected, onSelect, onOpenConversation }) => {
  const { t } = useTranslation();
  const meta = STATUS_META[run.status];
  const dotColor = meta?.color ?? 'var(--color-text-3)';
  const statusLabel = t(`orchestrator.run.status.${meta?.key ?? 'unknown'}`);
  const goalText = run.goal.trim() || t('orchestrator.run.untitledGoal');

  return (
    <div
      role='button'
      tabIndex={0}
      aria-pressed={selected}
      onClick={onSelect}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          onSelect();
        }
      }}
      className='group flex cursor-pointer select-none items-center gap-8px rd-10px px-12px py-10px transition-all duration-150'
      style={{
        background: selected ? 'color-mix(in srgb, rgb(var(--primary-6)) 8%, var(--bg-2))' : 'transparent',
        border: `1px solid ${selected ? 'rgb(var(--primary-6))' : 'transparent'}`,
        boxShadow: selected ? '0 0 0 3px color-mix(in srgb, rgb(var(--primary-6)) 16%, transparent)' : undefined,
      }}
    >
      <span
        className='size-7px shrink-0 rd-full'
        style={{ background: dotColor, boxShadow: `0 0 0 2px color-mix(in srgb, ${dotColor} 22%, transparent)` }}
      />
      <div className='min-w-0 flex-1'>
        <div className='truncate text-13px font-600 leading-tight text-t-primary'>{goalText}</div>
        <div className='mt-3px flex items-center gap-6px truncate text-11px text-t-tertiary'>
          <span className='shrink-0' style={{ color: dotColor }}>
            {statusLabel}
          </span>
          <span className='shrink-0'>·</span>
          <span className='truncate'>{formatTime(run.created_at)}</span>
        </div>
      </div>
      {onOpenConversation && (
        <div
          role='button'
          tabIndex={0}
          aria-label={t('orchestrator.run.openConversation')}
          title={t('orchestrator.run.openConversation')}
          onClick={(e) => {
            e.stopPropagation();
            onOpenConversation();
          }}
          onKeyDown={(e) => {
            if (e.key === 'Enter' || e.key === ' ') {
              e.preventDefault();
              e.stopPropagation();
              onOpenConversation();
            }
          }}
          className='flex size-26px shrink-0 items-center justify-center rd-6px text-t-tertiary opacity-0 transition-all hover:bg-fill-2 hover:text-primary-6 group-hover:opacity-100'
        >
          <Comment theme='outline' size='14' strokeWidth={3} />
        </div>
      )}
      <Right theme='outline' size='14' strokeWidth={3} className='shrink-0 text-t-tertiary' />
    </div>
  );
};

/**
 * RunListRail — the master column: a prominent 「＋ 新建 Run」button atop a
 * scrollable list of the current user's runs (active + history, newest first via
 * {@link useMyRuns}). Selecting a row is the page's primary navigation; the
 * 「新建」button and any open run live in the detail pane on the right.
 */
const RunListRail: React.FC<{
  selectedRunId: string | undefined;
  composing: boolean;
  onNewRun: () => void;
  onSelectRun: (id: string) => void;
}> = ({ selectedRunId, composing, onNewRun, onSelectRun }) => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { runs, isLoading, error } = useMyRuns();

  return (
    <div className='flex size-full min-h-0 w-300px shrink-0 flex-col border-r border-r-base bg-1'>
      {/* Header + new-run button */}
      <div className='shrink-0 px-16px pt-16px pb-12px'>
        <div className='text-15px font-600 leading-tight text-t-primary'>{t('orchestrator.title')}</div>
        <div className='mt-2px text-11px leading-15px text-t-tertiary'>{t('orchestrator.subtitle')}</div>
        <div
          role='button'
          tabIndex={0}
          aria-pressed={composing}
          onClick={onNewRun}
          onKeyDown={(e) => {
            if (e.key === 'Enter' || e.key === ' ') {
              e.preventDefault();
              onNewRun();
            }
          }}
          className='mt-12px flex h-36px cursor-pointer select-none items-center justify-center gap-6px rd-9px text-13px font-500 text-white transition-opacity hover:opacity-90'
          style={{
            background: 'rgb(var(--primary-6))',
            boxShadow: composing ? '0 0 0 3px color-mix(in srgb, rgb(var(--primary-6)) 22%, transparent)' : undefined,
          }}
        >
          <Plus theme='outline' size='15' strokeWidth={4} />
          <span>{t('orchestrator.tab.newRun')}</span>
        </div>
      </div>

      {/* List header */}
      <div className='shrink-0 px-16px pb-6px text-11px font-600 uppercase leading-none tracking-wide text-t-tertiary'>
        {t('orchestrator.tab.listTitle')}
      </div>

      {/* Scrollable run list */}
      <div className='min-h-0 flex-1 overflow-y-auto px-8px pb-12px'>
        {isLoading ? (
          <div className='flex items-center justify-center py-32px'>
            <Spin />
          </div>
        ) : error ? (
          <div className='px-8px py-24px text-center text-12px text-t-tertiary'>
            {t('orchestrator.tab.listLoadError')}
          </div>
        ) : runs.length === 0 ? (
          <div className='px-8px py-24px text-center text-12px leading-18px text-t-tertiary'>
            {t('orchestrator.tab.listEmpty')}
          </div>
        ) : (
          <div className='flex flex-col gap-2px'>
            {runs.map((run) => (
              <RunListRow
                key={run.id}
                run={run}
                selected={!composing && selectedRunId === run.id}
                onSelect={() => onSelectRun(run.id)}
                onOpenConversation={
                  run.lead_conv_id != null ? () => void navigate(`/conversation/${run.lead_conv_id}`) : undefined
                }
              />
            ))}
          </div>
        )}
      </div>
    </div>
  );
};

/** The clean empty state shown when nothing is selected and not composing. */
const EmptyDetail: React.FC = () => {
  const { t } = useTranslation();
  return (
    <div className='flex size-full min-h-0 flex-col items-center justify-center gap-14px px-24px text-center'>
      <span className='flex size-56px items-center justify-center rd-16px bg-fill-2 text-t-tertiary'>
        <Workbench theme='outline' size='28' strokeWidth={3} />
      </span>
      <div className='text-16px font-600 text-t-primary'>{t('orchestrator.empty.title')}</div>
      <div className='max-w-360px text-12px leading-18px text-t-tertiary'>{t('orchestrator.empty.desc')}</div>
    </div>
  );
};

/**
 * OrchestratorPage (/orchestrator) — 「智能编排」(orchestration), rebuilt as a
 * master-detail workspace. The left rail ({@link RunListRail}) lists the user's
 * runs with a prominent 「＋ 新建 Run」button; the detail pane has three states:
 *
 *  1. **composing** — {@link NewRunComposer} (after pressing 「＋ 新建 Run」).
 *     On `onCreated` we select the new run; on `onCancel` we drop back.
 *  2. **a run selected** (`?run=<id>`) — the run view: an {@link AgentRoster}
 *     strip atop the interactive {@link DagCanvas} (which itself renders the
 *     run-detail header + status-aware controls + the completed-run role
 *     precipitation panel, and wires cancel/approve/pause/resume internally —
 *     so we don't duplicate those here). Clicking a roster card or a DAG node
 *     opens the {@link WorkerTranscriptPanel} drawer.
 *  3. **nothing selected** — a clean {@link EmptyDetail} prompt.
 *
 * `?run=<id>` is kept in the URL (browser-back closes a run / a deep-link
 * selects one on mount). On mobile the interactive canvas is too awkward, so a
 * read-only {@link MobileRunSummary} (and the {@link RunHistory} list) is shown.
 */
const OrchestratorPage: React.FC = () => {
  const { t } = useTranslation();
  const layout = useLayoutContext();
  const isMobile = layout?.isMobile ?? false;
  const [searchParams, setSearchParams] = useSearchParams();

  // ── Master-detail state ────────────────────────────────────────────────────
  // `?run=<id>` is the source of truth for the selected run (deep-link + back).
  const runParam = searchParams.get('run');
  const selectedRunId = runParam && runParam !== '' ? runParam : undefined;

  const [composing, setComposing] = useState(false);
  // The clicked DAG node / roster card payload → opens the transcript drawer.
  const [selectedTask, setSelectedTask] = useState<OpenTaskPayload | null>(null);

  // Live run detail — fed to AgentRoster (DagCanvas self-fetches its own copy).
  // Called unconditionally with `undefined` when no run is selected (hooks rule).
  const { detail, refetch } = useRunLive(selectedRunId ?? undefined);

  // Selecting a run sets `?run=`; replace:false so browser-back closes it.
  const selectRun = useCallback(
    (id: string) => {
      setComposing(false);
      setSearchParams(
        (prev) => {
          const p = new URLSearchParams(prev);
          p.set('run', id);
          return p;
        },
        { replace: false }
      );
    },
    [setSearchParams]
  );

  const closeRun = useCallback(() => {
    setSearchParams(
      (prev) => {
        const p = new URLSearchParams(prev);
        p.delete('run');
        return p;
      },
      { replace: false }
    );
  }, [setSearchParams]);

  const startComposing = useCallback(() => {
    setComposing(true);
    closeRun();
  }, [closeRun]);

  // Closing the run (or leaving compose) dismisses any open transcript drawer.
  useEffect(() => {
    if (!selectedRunId) setSelectedTask(null);
  }, [selectedRunId]);

  // ── Mobile: read-only list / summary (no interactive canvas) ────────────────
  if (isMobile) {
    return (
      <div className='box-border min-h-full w-full overflow-y-auto px-16px py-16px'>
        <div className='text-20px font-600 leading-tight text-t-primary'>{t('orchestrator.title')}</div>
        <div className='mb-14px mt-4px text-12px leading-16px text-t-tertiary'>{t('orchestrator.subtitle')}</div>
        {selectedRunId ? (
          <MobileRunSummary runId={selectedRunId} onBack={closeRun} />
        ) : (
          <RunHistory onOpenRun={selectRun} />
        )}
      </div>
    );
  }

  return (
    <div className='relative flex size-full min-h-0'>
      <RunListRail
        selectedRunId={selectedRunId}
        composing={composing}
        onNewRun={startComposing}
        onSelectRun={selectRun}
      />

      {/* Detail pane — three states. */}
      <div className='relative flex min-h-0 min-w-0 flex-1 flex-col' role='tabpanel' aria-label={t('orchestrator.title')}>
        {composing ? (
          <div className='min-h-0 flex-1 overflow-y-auto px-40px py-32px'>
            <NewRunComposer
              onCreated={(runId) => {
                setComposing(false);
                selectRun(runId);
              }}
              onCancel={() => setComposing(false)}
            />
          </div>
        ) : selectedRunId ? (
          <>
            {/* AgentRoster sits above the canvas; DagCanvas brings its own header,
                run controls, and completed-run precipitation panel. */}
            {detail && (
              <AgentRoster
                detail={detail}
                selectedTaskId={selectedTask?.task.id ?? null}
                onSelectTask={setSelectedTask}
                refetch={refetch}
              />
            )}
            <div className='min-h-0 flex-1 overflow-hidden'>
              <Suspense fallback={<AppLoader />}>
                <DagCanvas runId={selectedRunId} onBack={closeRun} onOpenTask={setSelectedTask} />
              </Suspense>
            </div>
          </>
        ) : (
          <EmptyDetail />
        )}
      </div>

      {/* Task inspector + worker transcript drawer — always mounted, visible
          when a task node / roster card is clicked. */}
      <WorkerTranscriptPanel open={selectedTask} onClose={() => setSelectedTask(null)} />
    </div>
  );
};

export default OrchestratorPage;
