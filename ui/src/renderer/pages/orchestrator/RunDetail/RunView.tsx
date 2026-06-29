/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { Suspense, useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Branch, Comment } from '@icon-park/react';
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
import RunDecisionFeed, { type IntentTurn } from './RunDecisionFeed';
import RunIntentBox from './RunIntentBox';
import RunWorkspaceRail from './RunWorkspaceRail';
import type { OpenTaskPayload } from './DagCanvas';

// react-flow (heavy) is only needed inside the run view, so the canvas chunk is
// loaded on demand here just like the standalone page did.
const DagCanvas = React.lazy(() => import('./DagCanvas'));

/** localStorage key for the 对话 ⟷ 编排画布 view preference. Orchestrator-specific
 * so it never bleeds into any other surface's view state. */
const RUNVIEW_MODE_KEY = 'nomifun:orchestrator-runview-mode';
type RunViewMode = 'conversation' | 'canvas';

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
 * RunView — the run-detail workspace. Its main column carries a 对话 ⟷ 编排画布
 * segmented toggle (UC-4-convo) at the top, swapping the body between:
 *  • **对话** (default) — a {@link RunDecisionFeed} conversation thread that makes
 *    the lead agent's orchestration decisions legible (the structured decision +
 *    this session's intent-exchange turns), assembled frontend-only from the live
 *    detail;
 *  • **编排画布** — the existing {@link AgentRoster} strip atop the interactive
 *    {@link DagCanvas} (unchanged).
 * The {@link RunIntentBox} stays docked at the bottom in BOTH views (the shared
 * input). An optional preview column + a collapsible right rail
 * ({@link RunWorkspaceRail}, titlebar-toggled) are unaffected. Wrapped in a
 * run-scoped {@link PreviewProvider} so a rail file-click opens the preview
 * column (the worker-transcript drawer mounts its own provider, so this one is
 * isolated to `orchestrator-run`).
 */
const RunView: React.FC<RunViewProps> = (props) => (
  <PreviewProvider persistNamespace='orchestrator-run' subscribeGlobalOpen={false}>
    <RunViewInner {...props} />
  </PreviewProvider>
);

/** The 对话 ⟷ 编排画布 segmented control — a clean two-segment pill matching the
 * orchestrator visual language (primary-tinted active segment, theme tokens). */
const ViewToggle: React.FC<{ mode: RunViewMode; onChange: (mode: RunViewMode) => void }> = ({ mode, onChange }) => {
  const { t } = useTranslation();
  const segments: { key: RunViewMode; label: string; hint: string; Glyph: typeof Comment }[] = [
    {
      key: 'conversation',
      label: t('orchestrator.run.view.conversation'),
      hint: t('orchestrator.run.view.conversationHint'),
      Glyph: Comment,
    },
    { key: 'canvas', label: t('orchestrator.run.view.canvas'), hint: t('orchestrator.run.view.canvasHint'), Glyph: Branch },
  ];
  return (
    <div
      role='tablist'
      aria-label={t('orchestrator.title')}
      className='inline-flex shrink-0 items-center gap-2px rd-10px p-3px'
      style={{ background: 'var(--bg-2)', border: '1px solid var(--border-base)' }}
    >
      {segments.map(({ key, label, hint, Glyph }) => {
        const active = mode === key;
        return (
          <div
            key={key}
            role='tab'
            tabIndex={0}
            aria-selected={active}
            title={hint}
            onClick={() => onChange(key)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' || e.key === ' ') {
                e.preventDefault();
                onChange(key);
              }
            }}
            className='flex h-26px cursor-pointer select-none items-center gap-5px rd-8px px-12px text-12px font-600 leading-none outline-none transition-all duration-150'
            style={{
              background: active ? 'rgb(var(--primary-6))' : 'transparent',
              color: active ? '#fff' : 'var(--text-secondary)',
              boxShadow: active ? '0 1px 4px color-mix(in srgb, rgb(var(--primary-6)) 40%, transparent)' : undefined,
            }}
          >
            <Glyph theme='outline' size='13' strokeWidth={3} className='line-height-0' />
            <span>{label}</span>
          </div>
        );
      })}
    </div>
  );
};

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

  // ── 对话 ⟷ 编排画布 view toggle (UC-4-convo) ────────────────────────────────
  // Conversation-primary by default; persisted per the orchestrator-specific key.
  // The toggle swaps only the main-column body — the docked intent box, preview
  // column and workspace rail are unaffected.
  const [viewMode, setViewMode] = useState<RunViewMode>(readRunViewMode);
  const handleViewMode = useCallback((mode: RunViewMode) => {
    setViewMode(mode);
    try {
      localStorage.setItem(RUNVIEW_MODE_KEY, mode);
    } catch {
      // Best-effort persistence; an unavailable localStorage just won't remember.
    }
  }, []);

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

  return (
    <div className='flex size-full min-h-0'>
      {/* Main column: view toggle + (对话 feed | 编排画布) + docked intent box. */}
      <div className='flex min-h-0 min-w-0 flex-1 flex-col'>
        {/* View toggle — clean segmented control atop the main column. Only shown
            once the run detail has loaded (the empty/planning states render their
            own content inside the canvas). */}
        {detail && (
          <div className='flex shrink-0 items-center border-b border-b-base bg-1 px-16px py-8px'>
            <ViewToggle mode={viewMode} onChange={handleViewMode} />
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
                <DagCanvas runId={runId} onBack={onBack} onOpenTask={onSelectTask} onReplan={onReplan} />
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
