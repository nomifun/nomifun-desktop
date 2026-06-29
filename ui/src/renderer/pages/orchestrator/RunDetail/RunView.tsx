/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { Suspense } from 'react';
import { useTranslation } from 'react-i18next';
import type { TRunDetail } from '@/common/types/orchestrator/orchestratorTypes';
import AppLoader from '@/renderer/components/layout/AppLoader';
import { useLayoutContext } from '@/renderer/hooks/context/LayoutContext';
import { isDesktopShell, isMacOS, isWindows } from '@/renderer/utils/platform';
import { PreviewPanel, PreviewProvider, usePreviewContext } from '@/renderer/pages/conversation/Preview';
import { useWorkspaceCollapse } from '@/renderer/pages/conversation/hooks/useWorkspaceCollapse';
import WorkspacePanelHeader, {
  DesktopWorkspaceToggle,
} from '@/renderer/pages/conversation/components/ChatLayout/WorkspacePanelHeader';
import { WORKSPACE_HEADER_HEIGHT } from '@/renderer/pages/conversation/utils/layoutCalc';
import { dispatchWorkspaceToggleEvent } from '@/renderer/utils/workspace/workspaceEvents';
import AgentRoster from './AgentRoster';
import RunIntentBox from './RunIntentBox';
import RunWorkspaceRail from './RunWorkspaceRail';
import type { OpenTaskPayload } from './DagCanvas';

// react-flow (heavy) is only needed inside the run view, so the canvas chunk is
// loaded on demand here just like the standalone page did.
const DagCanvas = React.lazy(() => import('./DagCanvas'));

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
  const layout = useLayoutContext();
  const isMobile = Boolean(layout?.isMobile);
  const { isOpen: isPreviewOpen } = usePreviewContext();
  const workDir = detail?.run.work_dir?.trim() ?? '';
  const hasWorkDir = workDir.length > 0;

  // Desktop-shell mac/win runtime — gate on isDesktopShell() first (matching
  // ChatLayout/TerminalSessionPage): on mac/Windows the titlebar drives the
  // toggle, so the in-panel toggle + floating expand button are hidden there;
  // everyone else (Linux desktop, WebUI browser) keeps the in-panel toggle.
  const isDesktopRuntime = isDesktopShell();
  const isMacRuntime = isDesktopRuntime && isMacOS();
  const isWindowsRuntime = isDesktopRuntime && isWindows();

  // Rail collapse — the SAME hook the conversation / terminal rails use, so the
  // titlebar workspace button (WORKSPACE_TOGGLE_EVENT) toggles it and the
  // titlebar icon stays in sync (WORKSPACE_STATE_EVENT). Per-run preference key;
  // a run's work_dir is the user's own artifact dir (not a temp workspace), so
  // it auto-expands once the work_dir's files load. When the run has no work_dir
  // the hook stays force-collapsed and broadcasts collapsed STATE.
  const { rightSiderCollapsed } = useWorkspaceCollapse({
    workspaceEnabled: hasWorkDir,
    isMobile,
    preferenceKey: `orchestrator-run-${runId}`,
    isTemporaryWorkspace: false,
  });

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

        {/* Intent box (UC-3b) — the headline conversational surface: the user
            tells the orchestrator, in natural language, how to re-adjust the
            live plan; the main agent intelligently re-decomposes + re-drives,
            and a kept/新增/移除 summary reports what changed. Docked at the
            bottom of the main column so the DAG above stays the focal point.
            Only shown once the run detail has loaded. */}
        {detail && <RunIntentBox runId={runId} detail={detail} refetch={refetch} />}
      </div>

      {/* Preview column — opens when a rail file is clicked. */}
      {isPreviewOpen && (
        <div className='relative flex min-h-0 w-420px shrink-0 flex-col border-l border-l-base bg-1'>
          <PreviewPanel />
        </div>
      )}

      {/* Workspace rail (Files / Changes) — only when the run carries a work_dir
          (legacy workspace-backed runs without one simply omit the rail). The
          rail collapse is driven by the titlebar workspace toggle on mac /
          Windows / WebUI; Linux desktop keeps the in-panel toggle +
          DesktopWorkspaceToggle floating button (mirrors ChatLayout / terminal).
          Collapsed → width 0 (no slim strip), matching the conversation Tab. */}
      {hasWorkDir && detail && !isMobile && (
        <div
          className='!bg-1 relative shrink-0 layout-sider'
          style={{
            width: rightSiderCollapsed ? '0px' : '340px',
            minWidth: rightSiderCollapsed ? '0px' : '340px',
            overflow: 'hidden',
            borderLeft: rightSiderCollapsed ? 'none' : '1px solid var(--bg-3)',
          }}
        >
          <WorkspacePanelHeader
            showToggle={!isMacRuntime && !isWindowsRuntime}
            collapsed={rightSiderCollapsed}
            onToggle={() => dispatchWorkspaceToggleEvent()}
            togglePlacement='right'
            workspacePath={workDir}
          >
            <span className='truncate text-13px font-600 text-t-primary'>{t('orchestrator.run.workspace.title')}</span>
          </WorkspacePanelHeader>
          <div style={{ height: `calc(100% - ${WORKSPACE_HEADER_HEIGHT}px)` }}>
            <RunWorkspaceRail run={detail.run} />
          </div>
        </div>
      )}

      {/* Desktop expand button when collapsed — Linux/web only (mac/Windows use
          the titlebar workspace button). */}
      {hasWorkDir &&
        detail &&
        !isMacRuntime &&
        !isWindowsRuntime &&
        rightSiderCollapsed &&
        !isMobile && <DesktopWorkspaceToggle />}
    </div>
  );
};

export default RunView;
