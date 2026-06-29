/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { Suspense, useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Modal } from '@arco-design/web-react';
import { Loading, Minus } from '@icon-park/react';
import { ipcBridge } from '@/common';
import type { TReplanRequest } from '@/common/types/orchestrator/orchestratorTypes';
import AppLoader from '@/renderer/components/layout/AppLoader';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
// Reuse the conversation page's glass-header visual language (bg-1 92% + backdrop
// blur + gradient sink), so the overlay head reads identically to the chat head.
import '@/renderer/pages/conversation/components/ChatLayout/chat-layout.css';
import OrchestratorComposer, {
  type AutonomyLevel,
  type ComposerModelRange,
} from '@/renderer/pages/orchestrator/OrchestratorComposer';
import { useModelRange } from '@/renderer/pages/orchestrator/useModelRange';
import RunDecisionFeed, { type IntentTurn } from '@/renderer/pages/orchestrator/RunDetail/RunDecisionFeed';
import RunIntentBox from '@/renderer/pages/orchestrator/RunDetail/RunIntentBox';
import { RunControls } from '@/renderer/pages/orchestrator/RunDetail/RunControls';
import { RunTitleEditor } from '@/renderer/pages/orchestrator/RunDetail/RunTitleEditor';
import { ViewToggle, type RunViewMode } from '@/renderer/pages/orchestrator/RunDetail/ViewToggle';
import { STATUS_META } from '@/renderer/pages/orchestrator/RunDetail/runStatusMeta';
import { useOrchestration } from './OrchestrationContext';
import styles from './orchestrationCanvasOverlay.module.css';

// react-flow (heavy) only mounts when the canvas view is shown, so the canvas
// chunk is code-split here exactly like the standalone RunView did.
const DagCanvas = React.lazy(() => import('@/renderer/pages/orchestrator/RunDetail/DagCanvas'));

/**
 * Z-INDEX — the floating canvas must float ABOVE the conversation content (chat
 * messages, sider, header ≈ 0–100) but BELOW Arco's global Modal / Message
 * portals so the replan Modal and any toast still land on top. Arco's default
 * Modal mask z-index is 1000; we sit at 900 (panel) so the overlay clears the
 * conversation surface yet never occludes a Modal/Message it itself opens.
 */
const OVERLAY_Z = 900;

/** Default panel size (mirrors `.panel` in the module CSS) — used to seed the
 * initial centered-ish position and to clamp drags inside the viewport. */
const PANEL_W = 760;
const PANEL_H = 560;
/** Viewport gutter kept around the panel when clamping (matches the CSS max). */
const GUTTER = 12;

interface Point {
  x: number;
  y: number;
}

/** Clamp a top-left point so the panel stays fully inside the viewport. The
 * effective size is capped at the viewport (matching the CSS `max-*`), so a
 * window smaller than the panel still pins the panel to the gutter. */
function clampToViewport(p: Point): Point {
  const vw = window.innerWidth;
  const vh = window.innerHeight;
  const w = Math.min(PANEL_W, vw - GUTTER * 2);
  const h = Math.min(PANEL_H, vh - GUTTER * 2);
  const maxX = Math.max(GUTTER, vw - w - GUTTER);
  const maxY = Math.max(GUTTER, vh - h - GUTTER);
  return {
    x: Math.min(Math.max(p.x, GUTTER), maxX),
    y: Math.min(Math.max(p.y, GUTTER), maxY),
  };
}

/** Seed position — centered horizontally, biased toward the upper third so the
 * panel doesn't crowd the docked composer / bottom of the screen. */
function initialPosition(): Point {
  const vw = window.innerWidth;
  const vh = window.innerHeight;
  const w = Math.min(PANEL_W, vw - GUTTER * 2);
  return clampToViewport({ x: (vw - w) / 2, y: Math.max(GUTTER, vh * 0.12) });
}

/**
 * OrchestrationCanvasOverlay — the floating agent canvas for 「会话原生编排 v2」.
 * Reads the conversation's {@link useOrchestration} state and renders one of
 * three forms:
 *
 *  • `runId == null` → nothing (the conversation isn't linked to a run);
 *  • `canvasOpen` → a draggable, glass-headed floating panel that REPLICATES the
 *    standalone {@link RunView} composition — a glass header (inline-editable
 *    goal + status pill + 规划中 indicator on the left; {@link RunControls} +
 *    {@link ViewToggle} + a 收起 button on the right), a body that swaps between
 *    the lazy {@link DagCanvas} (画布) and the {@link RunDecisionFeed} (对话), and a
 *    docked {@link RunIntentBox} adjust composer. The header is the drag handle;
 *    每个 control `stopPropagation`s so a click never starts a drag;
 *  • collapsed (`!canvasOpen`) → a small bottom-right chip (status dot + truncated
 *    goal + planning pulse) that re-opens the panel.
 *
 * Lives as a sibling of ChatLayout inside the conversation's OrchestrationProvider
 * (mounted by NomiConversationPanel), so it never touches ChatLayout / NomiChat.
 */
const OrchestrationCanvasOverlay: React.FC = () => {
  const { t } = useTranslation();
  const {
    runId,
    detail,
    refetch,
    leadThinking,
    canvasOpen,
    openCanvas,
    collapseCanvas,
    projectTask,
    returnToMain,
    projectedTaskId,
  } = useOrchestration();
  const [message, msgCtx] = useArcoMessage();
  const { buildModelRange } = useModelRange();

  // ── 画布 ⟷ 对话 view (canvas-primary — the floating surface leads with the
  // canvas, unlike the standalone RunView's conversation-primary default). ──────
  const [viewMode, setViewMode] = useState<RunViewMode>('canvas');

  // Session intent-exchange turns — lifted here exactly like RunView: each intent
  // applied via RunIntentBox THIS session becomes a dialogue turn in the feed
  // (newest last). Reset when the run changes (a stale turn must not bleed across
  // runs). Persistence across reload is intentionally out of scope.
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

  // Inline rename → runs.rename (PATCH { goal }); refetch so the new goal lands
  // across the header + chip. A failure surfaces a toast (mirrors RunView).
  const handleRename = useCallback(
    async (goal: string) => {
      if (!runId) return;
      try {
        await ipcBridge.orchestrator.runs.rename.invoke({ id: runId, goal });
        await refetch();
      } catch (e) {
        message.error(t('orchestrator.run.manage.renameError', { error: String(e) }));
      }
    },
    [runId, refetch, message, t]
  );

  // ── Replan modal ────────────────────────────────────────────────────────────
  // v1 simplification: the replan composer prefills the run's goal + autonomy but
  // the model_range defaults to `auto` (every enabled pair) rather than being
  // reverse-rebuilt from the run's fleet_members snapshot. The user can narrow it
  // in the modal; reconstructing the prior fleet selection is left to a later pass.
  const [replanOpen, setReplanOpen] = useState(false);
  const [replanGoal, setReplanGoal] = useState('');
  const [replanModelRange, setReplanModelRange] = useState<ComposerModelRange>({
    mode: 'auto',
    single: '',
    range: [],
  });
  const [replanAutonomy, setReplanAutonomy] = useState<AutonomyLevel>('interactive');
  const [replanSubmitting, setReplanSubmitting] = useState(false);

  const openReplan = useCallback(() => {
    const goal = detail?.run.goal ?? '';
    setReplanGoal(goal);
    setReplanModelRange({ mode: 'auto', single: '', range: [] });
    setReplanAutonomy(detail?.run.autonomy === 'supervised' ? 'supervised' : 'interactive');
    setReplanOpen(true);
  }, [detail?.run.goal, detail?.run.autonomy]);

  const submitReplan = useCallback(
    async (goal: string) => {
      if (!runId) return;
      const trimmed = goal.trim();
      if (!trimmed) {
        message.warning(t('orchestrator.composer.goalRequired'));
        return;
      }
      const modelRange = buildModelRange({
        mode: replanModelRange.mode,
        single: replanModelRange.single,
        range: replanModelRange.range,
      });
      if (!modelRange) {
        message.warning(t('orchestrator.composer.modelRequired'));
        return;
      }
      setReplanSubmitting(true);
      try {
        const body: { id: string } & TReplanRequest = {
          id: runId,
          goal: trimmed,
          model_range: modelRange,
          autonomy: replanAutonomy,
        };
        await ipcBridge.orchestrator.runs.replan.invoke(body);
        message.success(t('orchestrator.run.detail.replanOk', { defaultValue: '已重新规划' }));
        await refetch();
        setReplanOpen(false);
      } catch (e) {
        message.error(t('orchestrator.composer.replanError', { error: String(e) }));
      } finally {
        setReplanSubmitting(false);
      }
    },
    [runId, buildModelRange, replanModelRange, replanAutonomy, refetch, message, t]
  );

  // ── Drag ──────────────────────────────────────────────────────────────────
  // Minimal pointer-event drag (no library: the repo has no position-drag hook,
  // only drag-upload / resize-split). The glass header is the handle; we capture
  // the pointer on it, track the offset from the panel's top-left, and clamp the
  // new position inside the viewport on every move + on resize.
  const [position, setPosition] = useState<Point>(initialPosition);
  const [dragging, setDragging] = useState(false);
  const dragOffset = useRef<Point>({ x: 0, y: 0 });

  // Recenter once when the panel (re)opens, so it lands somewhere sensible even
  // if the viewport changed while collapsed.
  useLayoutEffect(() => {
    if (canvasOpen) setPosition(initialPosition());
  }, [canvasOpen]);

  // Re-clamp on viewport resize so a shrunk window never strands the panel
  // partly off-screen.
  useEffect(() => {
    if (!canvasOpen) return undefined;
    const onResize = () => setPosition((p) => clampToViewport(p));
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  }, [canvasOpen]);

  const onHandlePointerDown = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      // Only the primary button drags; ignore secondary/middle.
      if (e.button !== 0) return;
      dragOffset.current = { x: e.clientX - position.x, y: e.clientY - position.y };
      setDragging(true);
      (e.currentTarget as HTMLDivElement).setPointerCapture(e.pointerId);
    },
    [position.x, position.y]
  );

  const onHandlePointerMove = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      if (!dragging) return;
      setPosition(
        clampToViewport({ x: e.clientX - dragOffset.current.x, y: e.clientY - dragOffset.current.y })
      );
    },
    [dragging]
  );

  const endDrag = useCallback((e: React.PointerEvent<HTMLDivElement>) => {
    setDragging(false);
    const el = e.currentTarget as HTMLDivElement;
    if (el.hasPointerCapture(e.pointerId)) el.releasePointerCapture(e.pointerId);
  }, []);

  // ── Render gates ────────────────────────────────────────────────────────────
  // No linked run → render nothing (don't even mount the chip).
  if (runId == null) return null;

  const status = detail?.run.status ?? '';
  const statusMeta = STATUS_META[status];
  const dotColor = statusMeta?.color ?? 'var(--color-text-3)';
  const statusLabel = t(`orchestrator.run.status.${statusMeta?.key ?? 'unknown'}`);
  const goalText = detail?.run.goal?.trim() || t('orchestrator.run.untitledGoal');

  // ── Collapsed chip ────────────────────────────────────────────────────────
  if (!canvasOpen) {
    return (
      <>
        {msgCtx}
        <div
          role='button'
          tabIndex={0}
          aria-label={t('orchestrator.canvas.openCanvas', { defaultValue: '展开 agent 画布' })}
          title={goalText}
          className={styles.chip}
          style={{ zIndex: OVERLAY_Z }}
          onClick={openCanvas}
          onKeyDown={(e) => {
            if (e.key === 'Enter' || e.key === ' ') {
              e.preventDefault();
              openCanvas();
            }
          }}
        >
          <span className={`${styles.chipDot} ${leadThinking.active ? styles.chipPulse : ''}`} style={{ background: dotColor }} />
          <span className={styles.chipLabel}>{goalText}</span>
          {leadThinking.active && (
            <Loading theme='outline' size='14' strokeWidth={3} className='shrink-0 animate-spin line-height-0' style={{ color: 'rgb(var(--primary-6))' }} />
          )}
        </div>
      </>
    );
  }

  // ── Expanded floating panel ──────────────────────────────────────────────
  return (
    <>
      {msgCtx}
      <div
        className={`${styles.panel} ${dragging ? styles.dragging : ''}`}
        style={{ left: position.x, top: position.y, zIndex: OVERLAY_Z }}
        role='dialog'
        aria-label={t('orchestrator.canvas.title', { defaultValue: 'Agent 画布' })}
      >
        {/* Glass header = drag handle. Mirrors RunView's head: left goal + status
            pill + 规划中; right RunControls + ViewToggle + 收起. */}
        <div
          className={`${styles.header} ${styles.dragHandle} chat-layout-header chat-layout-header--glass`}
          onPointerDown={onHandlePointerDown}
          onPointerMove={onHandlePointerMove}
          onPointerUp={endDrag}
          onPointerCancel={endDrag}
        >
          <div className='flex min-w-0 flex-1 items-center gap-10px'>
            <RunTitleEditor goal={detail?.run.goal ?? ''} onRename={handleRename} />
            <span
              className='inline-flex shrink-0 items-center gap-5px rd-full px-9px py-3px text-11px font-600 leading-none'
              style={{ color: dotColor, background: `color-mix(in srgb, ${dotColor} 12%, transparent)` }}
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
          {/* Right cluster — every interactive control stops the pointerdown from
              starting a drag (so a click on a button never drags the panel). */}
          <div
            className='flex shrink-0 items-center gap-10px'
            onPointerDown={(e) => e.stopPropagation()}
          >
            {detail && <RunControls runId={runId} status={status} refetch={refetch} onReplan={openReplan} />}
            <ViewToggle mode={viewMode} onChange={setViewMode} />
            <div
              role='button'
              tabIndex={0}
              aria-label={t('orchestrator.canvas.collapse', { defaultValue: '收起' })}
              title={t('orchestrator.canvas.collapse', { defaultValue: '收起' })}
              onClick={collapseCanvas}
              onKeyDown={(e) => {
                if (e.key === 'Enter' || e.key === ' ') {
                  e.preventDefault();
                  collapseCanvas();
                }
              }}
              className='flex size-28px shrink-0 cursor-pointer select-none items-center justify-center rd-8px border border-b-base text-t-secondary outline-none transition-all duration-150 hover:border-primary-6 hover:text-primary-6'
            >
              <Minus theme='outline' size='15' strokeWidth={3} />
            </div>
          </div>
        </div>

        {/* Body — 画布 (lazy DagCanvas) | 对话 (decision feed). Both keep the DAG
            chunk code-split; the feed never imports it. */}
        <div className={styles.body}>
          <div className={styles.viewport}>
            {viewMode === 'canvas' ? (
              <Suspense fallback={<AppLoader />}>
                <DagCanvas
                  runId={runId}
                  onOpenTask={projectTask}
                  onOpenMain={returnToMain}
                  mainActive={projectedTaskId === null}
                />
              </Suspense>
            ) : detail ? (
              <RunDecisionFeed
                detail={detail}
                turns={intentTurns}
                onSelectTask={projectTask}
                selectedTaskId={projectedTaskId}
                refetch={refetch}
              />
            ) : (
              <AppLoader />
            )}
          </div>

          {/* Docked adjust composer (UC-3b) — identical to RunView: an applied
              intent is also appended to the conversation feed as a turn. */}
          {detail && (
            <RunIntentBox runId={runId} detail={detail} refetch={refetch} onApplied={handleIntentApplied} />
          )}
        </div>
      </div>

      {/* Replan modal — the OrchestratorComposer (fluid) prefilled with the run's
          goal; model-range pill defaults to auto (v1 simplification — not rebuilt
          from the fleet snapshot) + the autonomy pill from the run. On submit →
          runs.replan → toast + refetch + close. Arco's Modal portals above the
          overlay (mask z 1000 > panel 900), so it lands on top correctly. */}
      <Modal
        title={t('orchestrator.run.detail.replan')}
        visible={replanOpen}
        footer={null}
        onCancel={() => {
          if (!replanSubmitting) setReplanOpen(false);
        }}
        maskClosable={!replanSubmitting}
        autoFocus={false}
        unmountOnExit
        style={{ width: 'min(640px, calc(100vw - 32px))' }}
      >
        <OrchestratorComposer
          fluid
          value={replanGoal}
          onChange={setReplanGoal}
          onSubmit={submitReplan}
          submitting={replanSubmitting}
          placeholder={t('orchestrator.composer.goalPlaceholder', { defaultValue: '描述要重新规划的目标…' })}
          label={t('orchestrator.run.detail.replan')}
          showModelRange
          modelRange={replanModelRange}
          onModelRangeChange={setReplanModelRange}
          showAutonomy
          autonomy={replanAutonomy}
          onAutonomyChange={setReplanAutonomy}
        />
      </Modal>
    </>
  );
};

export default OrchestrationCanvasOverlay;
