/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { Suspense, useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Loading } from '@icon-park/react';
import { ipcBridge } from '@/common';
import type { TRunDetail } from '@/common/types/orchestrator/orchestratorTypes';
import AppLoader from '@/renderer/components/layout/AppLoader';
import { useLayoutContext } from '@/renderer/hooks/context/LayoutContext';
import { isDesktopShell, isMacOS, isWindows } from '@/renderer/utils/platform';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import { PreviewPanel, PreviewProvider, usePreviewContext } from '@/renderer/pages/conversation/Preview';
import { useWorkspaceCollapse } from '@/renderer/pages/conversation/hooks/useWorkspaceCollapse';
import WorkspacePanelHeader, {
  DesktopWorkspaceToggle,
} from '@/renderer/pages/conversation/components/ChatLayout/WorkspacePanelHeader';
import { WORKSPACE_HEADER_HEIGHT } from '@/renderer/pages/conversation/utils/layoutCalc';
// Reuse the conversation page's glass-header visual language (bg-1 92% +
// backdrop blur + gradient sink). Importing the stylesheet here registers the
// `.chat-layout-header--glass` rules for this surface too.
import '@/renderer/pages/conversation/components/ChatLayout/chat-layout.css';
import { dispatchWorkspaceToggleEvent } from '@/renderer/utils/workspace/workspaceEvents';
import { useLeadThinking } from '../useLeadThinking';
import AgentRoster from './AgentRoster';
import RunDecisionFeed, { type IntentTurn } from './RunDecisionFeed';
import RunIntentBox from './RunIntentBox';
import RunWorkspaceRail from './RunWorkspaceRail';
import type { OpenTaskPayload } from './DagCanvas';
import { RunControls } from './RunControls';
import { RunTitleEditor } from './RunTitleEditor';
import { ViewToggle, type RunViewMode } from './ViewToggle';
import { STATUS_META } from './runStatusMeta';

// react-flow (heavy) is only needed inside the run view, so the canvas chunk is
// loaded on demand here just like the standalone page did.
const DagCanvas = React.lazy(() => import('./DagCanvas'));

/** localStorage key for the 对话 ⟷ 编排画布 view preference. Orchestrator-specific
 * so it never bleeds into any other surface's view state. */
const RUNVIEW_MODE_KEY = 'nomifun:orchestrator-runview-mode';

/** Read the persisted view mode, defaulting to the conversation-primary view. */
function readRunViewMode(): RunViewMode {
  try {
    return localStorage.getItem(RUNVIEW_MODE_KEY) === 'canvas' ? 'canvas' : 'conversation';
  } catch {
    return 'conversation';
  }
}

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
 * RunView — the run-detail workspace. Its main column is topped by a conversation-
 * style **glass header** (`chat-layout-header--glass`) shared across BOTH views:
 *  • left — the run goal as an inline-editable title (click to rename → `runs.rename`)
 *    plus a status pill (STATUS_META colors) and, while the lead agent is planning,
 *    a 「规划中」activity indicator (driven by {@link useLeadThinking});
 *  • right (headerExtra) — the status-gated run controls
 *    (approve / pause / resume / cancel, lifted up from {@link DagCanvas}) and the
 *    对话 ⟷ 编排画布 {@link ViewToggle}.
 * Below the header the body swaps between:
 *  • **对话** (default) — a {@link RunDecisionFeed} conversation thread;
 *  • **编排画布** — the {@link AgentRoster} strip atop the interactive
 *    {@link DagCanvas} (now canvas-only; its header/controls live in the glass head).
 * The {@link RunIntentBox} stays docked at the bottom in BOTH views. An optional
 * preview column + a collapsible right rail ({@link RunWorkspaceRail}) are
 * unaffected. Wrapped in a run-scoped {@link PreviewProvider}.
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
  onReplan,
}) => {
  const { t } = useTranslation();
  const layout = useLayoutContext();
  const isMobile = Boolean(layout?.isMobile);
  const { isOpen: isPreviewOpen } = usePreviewContext();
  const [message, msgCtx] = useArcoMessage();
  const workDir = detail?.run.work_dir?.trim() ?? '';
  const hasWorkDir = workDir.length > 0;

  // Live lead-agent planning indicator — when the main agent is mid-plan the
  // glass header shows a 「规划中」pulse (decoupled from the detail refetch).
  const leadThinking = useLeadThinking(detail ? runId : null);

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

  // ── 对话 ⟷ 编排画布 view toggle (UC-4-convo) ────────────────────────────────
  // Conversation-primary by default; persisted per the orchestrator-specific key.
  // The toggle swaps only the main-column body — the glass header, docked intent
  // box, preview column and workspace rail are unaffected.
  const [viewMode, setViewMode] = useState<RunViewMode>(readRunViewMode);
  const handleViewMode = useCallback((mode: RunViewMode) => {
    setViewMode(mode);
    try {
      localStorage.setItem(RUNVIEW_MODE_KEY, mode);
    } catch {
      // Best-effort persistence; an unavailable localStorage just won't remember.
    }
  }, []);

  // Inline rename → runs.rename (PATCH { goal }); refetch on success so the new
  // goal lands across the header + list. A failure surfaces a toast.
  const handleRename = useCallback(
    async (goal: string) => {
      try {
        await ipcBridge.orchestrator.runs.rename.invoke({ id: runId, goal });
        await refetch();
      } catch (e) {
        message.error(t('orchestrator.run.manage.renameError', { error: String(e) }));
      }
    },
    [runId, refetch, message, t]
  );

  // Session intent-exchange turns — each intent applied via RunIntentBox THIS
  // session becomes a dialogue turn in the conversation feed (newest last). Kept
  // in state here (lifted) so the feed shows the session's dialogue; persistence
  // across reload is intentionally out of scope (the current decision always
  // re-derives from the live detail). Reset when the run changes.
  const [intentTurns, setIntentTurns] = useState<IntentTurn[]>([]);
  useEffect(() => {
    setIntentTurns([]);
  }, [runId]);
  const handleIntentApplied = useCallback(
    (intent: string, summary: { kept: number; added: number; removed: number }) => {
      setIntentTurns((prev) => [...prev, { id: Date.now(), intent, summary }]);
    },
    []
  );

  const status = detail?.run.status ?? '';
  const statusMeta = STATUS_META[status];
  const dotColor = statusMeta?.color ?? 'var(--color-text-3)';
  const statusLabel = t(`orchestrator.run.status.${statusMeta?.key ?? 'unknown'}`);

  return (
    <div className='flex size-full min-h-0'>
      {msgCtx}
      {/* Main column: glass header + (对话 feed | 编排画布) + docked intent box. */}
      <div className='flex min-h-0 min-w-0 flex-1 flex-col'>
        {/* Conversation-style glass header — shared by BOTH views (only shown once
            the run detail has loaded; the planning empty-state renders inside the
            canvas). Left: inline-editable goal + status pill + 规划中 indicator.
            Right (headerExtra): run controls + 对话/画布 toggle. */}
        {detail && (
          <div
            className='min-h-44px flex shrink-0 items-center justify-between gap-16px overflow-hidden bg-1 px-16px pb-10px pt-8px chat-layout-header chat-layout-header--glass'
          >
            <div className='flex min-w-0 flex-1 items-center gap-10px'>
              <RunTitleEditor goal={detail.run.goal} onRename={handleRename} />
              <span
                className='inline-flex shrink-0 items-center gap-5px rd-full px-9px py-3px text-11px font-600 leading-none'
                style={{
                  color: dotColor,
                  background: `color-mix(in srgb, ${dotColor} 12%, transparent)`,
                }}
              >
                <span className='size-6px shrink-0 rd-full' style={{ background: dotColor }} />
                {statusLabel}
              </span>
              {leadThinking.active && (
                <span
                  className='inline-flex shrink-0 items-center gap-4px rd-full px-8px py-3px text-11px font-500 leading-none'
                  style={{
                    color: 'rgb(var(--primary-6))',
                    background: 'color-mix(in srgb, rgb(var(--primary-6)) 10%, transparent)',
                  }}
                >
                  <Loading theme='outline' size='12' strokeWidth={3} className='animate-spin line-height-0' />
                  {t('orchestrator.run.header.planning')}
                </span>
              )}
            </div>
            <div className='flex shrink-0 items-center gap-12px'>
              <RunControls runId={runId} status={status} refetch={refetch} onReplan={onReplan} />
              <ViewToggle mode={viewMode} onChange={handleViewMode} />
            </div>
          </div>
        )}

        {/* Body — swaps between the conversation feed and the roster + DAG. Both
            views keep the DAG chunk lazily code-split; the feed never imports it. */}
        {viewMode === 'conversation' && detail ? (
          <div className='min-h-0 flex-1 overflow-hidden'>
            <RunDecisionFeed
              detail={detail}
              turns={intentTurns}
              onSelectTask={onSelectTask}
              selectedTaskId={selectedTaskId}
              refetch={refetch}
            />
          </div>
        ) : (
          <>
            {detail && (
              <AgentRoster
                detail={detail}
                selectedTaskId={selectedTaskId}
                onSelectTask={onSelectTask}
                refetch={refetch}
              />
            )}
            <div className='min-h-0 flex-1 overflow-hidden'>
              <Suspense fallback={<AppLoader />}>
                <DagCanvas runId={runId} onOpenTask={onSelectTask} />
              </Suspense>
            </div>
          </>
        )}

        {/* Intent box (UC-3b) — the shared conversational input: the user tells
            the orchestrator, in natural language, how to re-adjust the live plan;
            the main agent intelligently re-decomposes + re-drives, and a
            kept/新增/移除 summary reports what changed. Docked at the bottom in
            BOTH views; an applied intent is also appended to the conversation feed
            as a dialogue turn. Only shown once the run detail has loaded. */}
        {detail && (
          <RunIntentBox runId={runId} detail={detail} refetch={refetch} onApplied={handleIntentApplied} />
        )}
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
