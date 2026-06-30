/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { Suspense, useCallback, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Down, Loading, Up } from '@icon-park/react';
import { Modal, Spin } from '@arco-design/web-react';
import { ipcBridge } from '@/common';
import type { TReplanRequest } from '@/common/types/orchestrator/orchestratorTypes';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import OrchestratorComposer, { type AutonomyLevel, type ComposerModelRange } from '@/renderer/pages/orchestrator/OrchestratorComposer';
import { useModelRange } from '@/renderer/pages/orchestrator/useModelRange';
import { RunControls } from '@/renderer/pages/orchestrator/RunDetail/RunControls';
import { STATUS_META } from '@/renderer/pages/orchestrator/RunDetail/runStatusMeta';
import { useOrchestration } from './OrchestrationContext';
import styles from './orchestrationTopPanel.module.css';

/**
 * Lazy-load the react-flow DAG canvas so its heavy graph deps (`@xyflow/react`)
 * aren't pulled into the conversation page bundle until a run actually exists to
 * preview in the top panel.
 */
const DagCanvas = React.lazy(() => import('@/renderer/pages/orchestrator/RunDetail/DagCanvas'));

/** Fallback color for an unknown run status — neutral tertiary text var. */
const STATUS_FALLBACK_COLOR = 'var(--color-text-3)';

/**
 * OrchestrationTopPanel — the orchestration canvas as a COLLAPSIBLE panel pinned
 * to the TOP of the conversation content area (用户定案 Option B). The main agent
 * chat coexists BELOW it (rendered by the content-area switcher). This is the
 * orchestration surface now — there is no floating overlay and no right-rail tab.
 *
 * Reads {@link useOrchestration} (always inside an `OrchestrationProvider` on the
 * nomi conversation surface):
 *  - `runId == null` → renders nothing (the panel only exists while a run is
 *    linked to this conversation; initiation lives on the homepage 模式条 + the
 *    main agent's own autonomy, NOT here).
 *  - has run → a header (always visible while a run exists) with a ▼/▲ collapse
 *    toggle, the「编排画布」title, the status pill ({@link STATUS_META}), a
 *    「规划中…」hint while the lead agent is still planning, and the compact
 *    {@link RunControls} (approve / pause / resume / cancel / replan). When
 *    `expanded` (default true), a height-bounded body hosts the lazy
 *    {@link DagCanvas} (node → `projectTask`, main → `returnToMain`,
 *    `mainActive = projectedTaskId === null`); the completion-state
 *    RolePrecipitationPanel is rendered by the canvas itself.
 *  - replan: RunControls' `onReplan` opens a standard Arco Modal (not a floating
 *    window) hosting the {@link OrchestratorComposer} (fluid) prefilled with the
 *    run's goal → `runs.replan` → toast + refetch + close.
 */
const OrchestrationTopPanel: React.FC = () => {
  const { t } = useTranslation();
  const [message, msgCtx] = useArcoMessage();
  const orchestration = useOrchestration();
  const { buildModelRange } = useModelRange();

  // Default EXPANDED for discoverability; collapsing leaves just the header strip.
  const [expanded, setExpanded] = useState(true);

  // ── Replan modal state ──────────────────────────────────────────────────────
  // v1 simplification: the replan composer prefills the run's goal + autonomy,
  // but the model_range defaults to `auto` (every enabled pair) rather than
  // being reverse-rebuilt from the run's fleet_members snapshot. The user can
  // narrow it in the modal.
  const [replanOpen, setReplanOpen] = useState(false);
  const [replanGoal, setReplanGoal] = useState('');
  const [replanModelRange, setReplanModelRange] = useState<ComposerModelRange>({ mode: 'auto', single: '', range: [] });
  const [replanAutonomy, setReplanAutonomy] = useState<AutonomyLevel>('interactive');
  const [replanSubmitting, setReplanSubmitting] = useState(false);

  const { runId, detail, leadThinking, refetch, projectTask, returnToMain, projectedTaskId } = orchestration;

  const openReplan = useCallback(() => {
    const goal = orchestration.detail?.run.goal ?? '';
    setReplanGoal(goal);
    setReplanModelRange({ mode: 'auto', single: '', range: [] });
    setReplanAutonomy(orchestration.detail?.run.autonomy === 'supervised' ? 'supervised' : 'interactive');
    setReplanOpen(true);
  }, [orchestration.detail?.run.goal, orchestration.detail?.run.autonomy]);

  const submitReplan = useCallback(
    async (goal: string) => {
      if (!runId) return;
      const trimmed = goal.trim();
      if (!trimmed) {
        message.warning(t('orchestrator.composer.goalRequired'));
        return;
      }
      const wireRange = buildModelRange({
        mode: replanModelRange.mode,
        single: replanModelRange.single,
        range: replanModelRange.range,
      });
      if (!wireRange) {
        message.warning(t('orchestrator.composer.modelRequired'));
        return;
      }
      setReplanSubmitting(true);
      try {
        const body: { id: string } & TReplanRequest = {
          id: runId,
          goal: trimmed,
          model_range: wireRange,
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

  // No run linked to this conversation → the panel does not exist.
  if (runId == null) return null;

  const status = detail?.run.status ?? '';
  const statusMeta = STATUS_META[status];
  const statusColor = statusMeta?.color ?? STATUS_FALLBACK_COLOR;
  const statusLabel = statusMeta
    ? t(`orchestrator.run.status.${statusMeta.key}`, { defaultValue: status })
    : t('orchestrator.run.status.unknown', { defaultValue: status });

  return (
    <div className={`${styles.panel} shrink-0 flex flex-col`}>
      {msgCtx}

      {/* Header — always shown while a run exists. Collapse toggle + title +
          status pill + planning hint on the left; compact RunControls on the
          right (allowed to wrap on a narrow content area). */}
      <div className={`${styles.header} flex flex-wrap items-center gap-x-10px gap-y-6px`}>
        <div
          role='button'
          tabIndex={0}
          aria-label={t('conversation.orchestration.panelTitle', { defaultValue: '编排画布' })}
          aria-expanded={expanded}
          onClick={() => setExpanded((v) => !v)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' || e.key === ' ') {
              e.preventDefault();
              setExpanded((v) => !v);
            }
          }}
          className={`${styles.toggle} inline-flex items-center gap-6px cursor-pointer select-none`}
        >
          {expanded ? (
            <Down theme='outline' size='14' strokeWidth={3} />
          ) : (
            <Up theme='outline' size='14' strokeWidth={3} />
          )}
          <span className='text-13px font-600 text-t-primary'>
            {t('conversation.orchestration.panelTitle', { defaultValue: '编排画布' })}
          </span>
        </div>

        <span
          className='inline-flex items-center gap-6px rd-full px-9px py-3px text-11px font-600 leading-none'
          style={{
            color: statusColor,
            background: 'color-mix(in srgb, currentColor 12%, transparent)',
          }}
        >
          <span className='size-6px rd-full shrink-0' style={{ background: statusColor }} />
          <span className='truncate'>{statusLabel}</span>
        </span>

        {leadThinking.active && (
          <span className='inline-flex items-center gap-5px text-11px text-primary-6 leading-none'>
            <Loading theme='outline' size='12' strokeWidth={3} className='animate-spin line-height-0' />
            <span>{t('conversation.orchestration.planning', { defaultValue: '规划中…' })}</span>
          </span>
        )}

        <div className='ml-auto'>
          <RunControls runId={runId} status={status} refetch={refetch} onReplan={openReplan} />
        </div>
      </div>

      {/* Body — only mounted when expanded. Height-bounded box so the react-flow
          canvas has a finite layout area while the chat below keeps `flex-1`. */}
      {expanded && (
        <div className={`${styles.body} min-h-0`}>
          <Suspense
            fallback={
              <div className='size-full flex items-center justify-center'>
                <Spin />
              </div>
            }
          >
            <DagCanvas
              runId={runId}
              onOpenTask={projectTask}
              onOpenMain={returnToMain}
              mainActive={projectedTaskId === null}
            />
          </Suspense>
        </div>
      )}

      {/* Replan modal — a STANDARD Arco dialog (not a floating window): the
          OrchestratorComposer (fluid) prefilled with the run's goal; model-range
          defaults to auto (v1 simplification — not rebuilt from the fleet
          snapshot) + the autonomy pill from the run. On submit → runs.replan →
          toast + refetch + close. */}
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
    </div>
  );
};

export default OrchestrationTopPanel;
