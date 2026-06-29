/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { Suspense, useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import classNames from 'classnames';
import { Input, Popconfirm } from '@arco-design/web-react';
import { Branch, CheckOne, Comment, Loading, Pause, PauseOne, PlayOne, Refresh } from '@icon-park/react';
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

// react-flow (heavy) is only needed inside the run view, so the canvas chunk is
// loaded on demand here just like the standalone page did.
const DagCanvas = React.lazy(() => import('./DagCanvas'));

/** Run status → theme-var color + i18n key suffix (mirrors the page's STATUS_META
 * + RunHistory). Single source of truth for the glass-header status pill. */
const STATUS_META: Record<string, { color: string; key: string }> = {
  planning: { color: 'var(--warning)', key: 'planning' },
  running: { color: 'rgb(var(--primary-6))', key: 'running' },
  completed: { color: 'var(--success)', key: 'completed' },
  failed: { color: 'var(--danger)', key: 'failed' },
  cancelled: { color: 'var(--color-text-3)', key: 'cancelled' },
  paused: { color: 'var(--warning)', key: 'paused' },
  awaiting_plan_approval: { color: 'rgb(var(--primary-6))', key: 'awaiting_plan_approval' },
};

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

/**
 * RunTitleEditor — the run goal rendered as an inline-editable title, modeled on
 * the conversation page's {@link ChatTitleEditor} (hover-revealed edit affordance,
 * click → in-place Arco Input, Enter commits / Escape cancels / blur commits).
 * Never a bare `<button>`: the resting state is a `role="button"` span. Commits
 * route through {@link ipcBridge.orchestrator.runs.rename} (PATCH `{ goal }`).
 */
const RunTitleEditor: React.FC<{
  goal: string;
  onRename: (goal: string) => Promise<void>;
}> = ({ goal, onRename }) => {
  const { t } = useTranslation();
  const goalText = goal.trim() || t('orchestrator.run.untitledGoal');

  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(goal);
  const [saving, setSaving] = useState(false);

  const beginEdit = useCallback(() => {
    setDraft(goal);
    setEditing(true);
  }, [goal]);

  const commitEdit = useCallback(async () => {
    const next = draft.trim();
    if (!next || next === goal.trim()) {
      setEditing(false);
      return;
    }
    setSaving(true);
    try {
      await onRename(next);
      setEditing(false);
    } finally {
      setSaving(false);
    }
  }, [draft, goal, onRename]);

  if (editing) {
    return (
      <div
        className='flex min-w-0 max-w-full flex-1 items-center rounded-12px border border-solid bg-fill-2 shadow-[0_1px_2px_rgba(15,23,42,0.06)]'
        style={{ borderColor: 'var(--color-fill-3)' }}
      >
        <div className='min-w-0 flex-1 px-8px py-3px'>
          <Input
            autoFocus
            value={draft}
            disabled={saving}
            maxLength={200}
            size='default'
            placeholder={t('orchestrator.run.header.renamePlaceholder')}
            className='w-full min-w-0 max-w-full border-none bg-transparent shadow-none [&_.arco-input-inner-wrapper]:border-none [&_.arco-input-inner-wrapper]:bg-transparent [&_.arco-input-inner-wrapper]:shadow-none [&_.arco-input]:bg-transparent [&_.arco-input]:px-0 [&_.arco-input]:text-15px [&_.arco-input]:font-600 [&_.arco-input]:leading-22px [&_.arco-input]:text-[var(--color-text-1)]'
            onChange={setDraft}
            onFocus={(event) => event.target.select()}
            onPressEnter={() => void commitEdit()}
            onBlur={() => void commitEdit()}
            onKeyDown={(event) => {
              if (event.key === 'Escape') {
                event.preventDefault();
                setDraft(goal);
                setEditing(false);
              }
            }}
          />
        </div>
      </div>
    );
  }

  return (
    <div className='group flex min-w-0 max-w-full flex-1 items-center rounded-12px border border-solid border-transparent transition-all duration-180 hover:bg-fill-2 hover:border-[var(--color-fill-3)] hover:shadow-[0_1px_2px_rgba(15,23,42,0.06)] focus-within:bg-fill-2 focus-within:border-[var(--color-fill-3)]'>
      <div className='min-w-0 flex-1 px-8px py-3px'>
        <span
          role='button'
          tabIndex={0}
          title={t('orchestrator.run.header.rename')}
          className='block min-w-0 cursor-text overflow-hidden text-ellipsis whitespace-nowrap text-15px font-600 leading-22px text-t-primary transition-colors duration-150 outline-none group-hover:text-[rgb(var(--primary-6))] group-focus-within:text-[rgb(var(--primary-6))]'
          onClick={beginEdit}
          onKeyDown={(event) => {
            if (event.key === 'Enter' || event.key === ' ') {
              event.preventDefault();
              beginEdit();
            }
          }}
        >
          {goalText}
        </span>
      </div>
    </div>
  );
};

/** A single status-gated header control. Never a bare `<button>` — a
 * `role="button"` div, busy-aware (greyed + click-suppressed while in flight). */
const HeaderControl: React.FC<{
  label: string;
  onClick: () => void;
  busy: boolean;
  tone?: 'primary' | 'neutral' | 'danger';
  children: React.ReactNode;
}> = ({ label, onClick, busy, tone = 'neutral', children }) => {
  const primary = tone === 'primary';
  const hover =
    tone === 'danger'
      ? 'hover:border-danger hover:text-danger'
      : tone === 'primary'
        ? 'hover:opacity-90'
        : 'hover:border-primary-6 hover:text-primary-6';
  return (
    <div
      role='button'
      tabIndex={0}
      aria-label={label}
      aria-disabled={busy}
      onClick={busy ? undefined : onClick}
      onKeyDown={(e) => {
        if (busy) return;
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          onClick();
        }
      }}
      className={classNames(
        'flex h-30px shrink-0 cursor-pointer select-none items-center gap-5px rd-8px px-10px text-12px font-500 transition-all duration-150',
        primary ? 'text-white' : 'border border-b-base text-t-secondary',
        hover
      )}
      style={{
        background: primary ? 'rgb(var(--primary-6))' : undefined,
        opacity: busy ? 0.6 : undefined,
        pointerEvents: busy ? 'none' : undefined,
      }}
    >
      {children}
      <span>{label}</span>
    </div>
  );
};

/**
 * RunControls — the status-aware run-control button group, lifted UP from the
 * DAG canvas into the shared glass header so it is reachable from BOTH the 对话
 * and 编排画布 views (and rendered exactly once). Status gating mirrors the old
 * run-detail header: awaiting → approve (primary); running → pause; paused → resume;
 * any non-terminal run → confirm-guarded cancel. Each action calls its REST
 * endpoint, toasts via {@link useArcoMessage}, then refetches.
 */
const RunControls: React.FC<{
  runId: string;
  status: string;
  refetch: () => Promise<void>;
  onReplan: () => void;
}> = ({ runId, status, refetch, onReplan }) => {
  const { t } = useTranslation();
  const [message, msgCtx] = useArcoMessage();
  const [busy, setBusy] = useState(false);

  const isTerminal = status === 'completed' || status === 'failed' || status === 'cancelled';

  const run = useCallback(
    async (
      action: () => Promise<void>,
      okKey: string,
      errKey: string,
    ) => {
      setBusy(true);
      try {
        await action();
        message.success(t(okKey));
        await refetch();
      } catch (e) {
        message.error(t(errKey, { error: String(e) }));
      } finally {
        setBusy(false);
      }
    },
    [message, refetch, t]
  );

  const onApprove = () =>
    void run(
      () => ipcBridge.orchestrator.runs.approve.invoke({ id: runId }),
      'orchestrator.run.detail.approveOk',
      'orchestrator.run.detail.approveError'
    );
  const onPause = () =>
    void run(
      () => ipcBridge.orchestrator.runs.pause.invoke({ id: runId }),
      'orchestrator.run.detail.pauseOk',
      'orchestrator.run.detail.pauseError'
    );
  const onResume = () =>
    void run(
      () => ipcBridge.orchestrator.runs.resume.invoke({ id: runId }),
      'orchestrator.run.detail.resumeOk',
      'orchestrator.run.detail.resumeError'
    );
  const onCancel = () =>
    void run(
      () => ipcBridge.orchestrator.runs.cancel.invoke({ id: runId }),
      'orchestrator.run.detail.cancelOk',
      'orchestrator.run.detail.cancelError'
    );

  return (
    <div className='flex shrink-0 items-center gap-8px'>
      {msgCtx}
      <HeaderControl label={t('orchestrator.run.detail.replan')} onClick={onReplan} busy={busy}>
        <Refresh theme='outline' size='14' strokeWidth={3} />
      </HeaderControl>
      {status === 'awaiting_plan_approval' && (
        <HeaderControl label={t('orchestrator.run.detail.approvePlan')} onClick={onApprove} busy={busy} tone='primary'>
          <CheckOne theme='outline' size='14' strokeWidth={3} />
        </HeaderControl>
      )}
      {status === 'running' && (
        <HeaderControl label={t('orchestrator.run.detail.pause')} onClick={onPause} busy={busy}>
          <PauseOne theme='outline' size='14' strokeWidth={3} />
        </HeaderControl>
      )}
      {status === 'paused' && (
        <HeaderControl label={t('orchestrator.run.detail.resume')} onClick={onResume} busy={busy}>
          <PlayOne theme='outline' size='14' strokeWidth={3} />
        </HeaderControl>
      )}
      {!isTerminal && (
        <Popconfirm
          focusLock
          title={t('orchestrator.run.detail.cancelConfirm')}
          okText={t('orchestrator.run.detail.cancelConfirmOk')}
          cancelText={t('orchestrator.run.detail.cancelConfirmCancel')}
          onOk={onCancel}
        >
          {/* Popconfirm needs a single focusable child; the control is busy-aware. */}
          <div
            role='button'
            tabIndex={0}
            aria-label={t('orchestrator.run.detail.cancel')}
            aria-disabled={busy}
            className='flex h-30px shrink-0 cursor-pointer select-none items-center gap-5px rd-8px border border-b-base px-10px text-12px font-500 text-t-secondary transition-all duration-150 hover:border-danger hover:text-danger'
            style={{ opacity: busy ? 0.6 : undefined, pointerEvents: busy ? 'none' : undefined }}
          >
            <Pause theme='outline' size='14' strokeWidth={3} />
            <span>{t('orchestrator.run.detail.cancel')}</span>
          </div>
        </Popconfirm>
      )}
    </div>
  );
};

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
