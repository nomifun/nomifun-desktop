/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { Suspense, useCallback, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ExpandRight, FolderClose } from '@icon-park/react';
import type { TRunDetail } from '@/common/types/orchestrator/orchestratorTypes';
import AppLoader from '@/renderer/components/layout/AppLoader';
import { PreviewPanel, PreviewProvider, usePreviewContext } from '@/renderer/pages/conversation/Preview';
import AgentRoster from './AgentRoster';
import RunWorkspaceRail from './RunWorkspaceRail';
import type { OpenTaskPayload } from './DagCanvas';

// react-flow (heavy) is only needed inside the run view, so the canvas chunk is
// loaded on demand here just like the standalone page did.
const DagCanvas = React.lazy(() => import('./DagCanvas'));

/** localStorage key for the run-view workspace rail collapse preference. */
const RUNWS_COLLAPSE_KEY = 'orchestrator-runworkspace-collapsed';

export interface RunViewProps {
  runId: string;
  /** Live run detail (drives the roster + the right-rail work_dir binding). */
  detail: TRunDetail | null | undefined;
  selectedTaskId: string | null;
  onSelectTask: (payload: OpenTaskPayload) => void;
  refetch: () => Promise<void>;
  onBack: () => void;
  onReplan: () => void;
}

/**
 * RunView — the run-detail workspace: an {@link AgentRoster} strip atop the
 * interactive {@link DagCanvas} on the left, an optional preview column, and a
 * collapsible right rail ({@link RunWorkspaceRail}) showing the run's work_dir
 * Files / Changes. Wrapped in a run-scoped {@link PreviewProvider} so a rail
 * file-click opens the preview column (the worker-transcript drawer mounts its
 * own provider, so this one is isolated to `orchestrator-run`).
 */
const RunView: React.FC<RunViewProps> = (props) => (
  <PreviewProvider persistNamespace='orchestrator-run' subscribeGlobalOpen={false}>
    <RunViewInner {...props} />
  </PreviewProvider>
);

const RunViewInner: React.FC<RunViewProps> = ({
  runId,
  detail,
  selectedTaskId,
  onSelectTask,
  refetch,
  onBack,
  onReplan,
}) => {
  const { t } = useTranslation();
  const { isOpen: isPreviewOpen } = usePreviewContext();
  const workDir = detail?.run.work_dir?.trim() ?? '';
  const hasWorkDir = workDir.length > 0;

  const [collapsed, setCollapsed] = useState<boolean>(() => {
    try {
      return localStorage.getItem(RUNWS_COLLAPSE_KEY) === '1';
    } catch {
      return false;
    }
  });
  const toggleCollapsed = useCallback(() => {
    setCollapsed((prev) => {
      const next = !prev;
      try {
        localStorage.setItem(RUNWS_COLLAPSE_KEY, next ? '1' : '0');
      } catch {
        /* ignore persistence failures */
      }
      return next;
    });
  }, []);

  const onKeyActivate = (fn: () => void) => (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      fn();
    }
  };

  return (
    <div className='flex size-full min-h-0'>
      {/* Main column: roster + DAG canvas. */}
      <div className='flex min-h-0 min-w-0 flex-1 flex-col'>
        {detail && (
          <AgentRoster detail={detail} selectedTaskId={selectedTaskId} onSelectTask={onSelectTask} refetch={refetch} />
        )}
        <div className='min-h-0 flex-1 overflow-hidden'>
          <Suspense fallback={<AppLoader />}>
            <DagCanvas runId={runId} onBack={onBack} onOpenTask={onSelectTask} onReplan={onReplan} />
          </Suspense>
        </div>
      </div>

      {/* Preview column — opens when a rail file is clicked. */}
      {isPreviewOpen && (
        <div className='relative flex min-h-0 w-420px shrink-0 flex-col border-l border-l-base bg-1'>
          <PreviewPanel />
        </div>
      )}

      {/* Workspace rail (Files / Changes) — only when the run carries a work_dir
          (legacy workspace-backed runs without one simply omit the rail). */}
      {hasWorkDir &&
        detail &&
        (collapsed ? (
          <div className='flex h-full w-44px shrink-0 flex-col items-center border-l border-l-base bg-1 py-12px'>
            <div
              role='button'
              tabIndex={0}
              aria-label={t('orchestrator.run.workspace.expand')}
              title={t('orchestrator.run.workspace.expand')}
              onClick={toggleCollapsed}
              onKeyDown={onKeyActivate(toggleCollapsed)}
              className='flex size-28px items-center justify-center rd-8px text-t-tertiary transition-colors hover:bg-fill-2 hover:text-t-primary'
            >
              <FolderClose theme='outline' size='16' strokeWidth={3} />
            </div>
          </div>
        ) : (
          <div className='flex h-full w-340px shrink-0 flex-col border-l border-l-base bg-1'>
            <div className='flex shrink-0 items-center justify-between border-b border-b-base px-12px py-10px'>
              <span className='truncate text-13px font-600 text-t-primary'>{t('orchestrator.run.workspace.title')}</span>
              <div
                role='button'
                tabIndex={0}
                aria-label={t('orchestrator.run.workspace.collapse')}
                title={t('orchestrator.run.workspace.collapse')}
                onClick={toggleCollapsed}
                onKeyDown={onKeyActivate(toggleCollapsed)}
                className='flex size-26px shrink-0 items-center justify-center rd-7px text-t-tertiary transition-colors hover:bg-fill-2 hover:text-t-primary'
              >
                <ExpandRight theme='outline' size='15' strokeWidth={3} />
              </div>
            </div>
            <div className='min-h-0 flex-1'>
              <RunWorkspaceRail run={detail.run} />
            </div>
          </div>
        ))}
    </div>
  );
};

export default RunView;
