/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { Suspense, useCallback, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Branch, Loading, Workbench } from '@icon-park/react';
import { Modal, Spin } from '@arco-design/web-react';
import { ipcBridge } from '@/common';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import type { TCreateAdhocRun, TReplanRequest } from '@/common/types/orchestrator/orchestratorTypes';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import OrchestratorComposer, {
  type AutonomyLevel,
  type ComposerModelRange,
} from '@/renderer/pages/orchestrator/OrchestratorComposer';
import { useModelRange } from '@/renderer/pages/orchestrator/useModelRange';
import { RunControls } from '@/renderer/pages/orchestrator/RunDetail/RunControls';
import { STATUS_META } from '@/renderer/pages/orchestrator/RunDetail/runStatusMeta';
import { useOrchestrationSafe } from './OrchestrationContext';

/**
 * Lazy-load the react-flow DAG canvas so its heavy graph deps (`@xyflow/react`)
 * aren't pulled into the conversation page bundle until the orchestration tab
 * actually has a run to preview.
 */
const DagCanvas = React.lazy(() => import('@/renderer/pages/orchestrator/RunDetail/DagCanvas'));

/** Fallback color for an unknown run status — neutral tertiary text var (mirrors
 * the glass-header pill's own fallback). */
const STATUS_FALLBACK_COLOR = 'var(--color-text-3)';

/**
 * OrchestrationRailTab — the conversation right-rail「编排」tab (会话原生编排 v2).
 *
 * This IS the orchestration surface now — there is no floating overlay. The
 * canvas + run controls live here in the rail; the conversation content area
 * keeps the native NomiChat + the node→content-area projection (F7) untouched.
 *
 * Two states, both reading {@link useOrchestrationSafe}:
 *  - **has run** (`runId != null`): a top control row (status pill colored from
 *    {@link STATUS_META}, a「规划中…」hint while the lead agent is still planning,
 *    and the compact {@link RunControls} — approve / pause / resume / cancel /
 *    replan; the row is allowed to wrap in the narrow rail), a replan Arco Modal
 *    (a standard dialog, not a floating window) hosting the {@link OrchestratorComposer}
 *    prefilled with the run's goal, and the lazy {@link DagCanvas} FILLING the rail
 *    height (click a node → `projectTask`, the main node → `returnToMain`,
 *    `mainActive = projectedTaskId === null`).
 *  - **no run** (`runId == null`): an initiation card whose {@link OrchestratorComposer}
 *    (in `fluid` mode so it fills the narrow rail) launches a Path-B ad-hoc run
 *    bound to the current conversation. We DON'T set `runId` after create — the
 *    backend writes `extra.orchestrator_run_id` + broadcasts `conversation.listChanged`,
 *    the conversation refetches, and `useConversationRun` lights up `runId` on its own.
 *
 * Reads via the SAFE hook because the companion「聊天」tab renders a `nomi`
 * conversation through `ChatSlider` WITHOUT an `OrchestrationProvider` — there
 * the tab degrades to a neutral empty state instead of throwing.
 */
const OrchestrationRailTab: React.FC = () => {
  const { t } = useTranslation();
  const [message, msgCtx] = useArcoMessage();
  const orchestration = useOrchestrationSafe();
  const { hasModels, buildModelRange } = useModelRange();

  // Path-B composer state (no-run initiation) — intent text + model range
  // (defaults to「auto」= every enabled model) + autonomy (defaults to
  // interactive: review the plan).
  const [intent, setIntent] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const [modelRange, setModelRange] = useState<ComposerModelRange>({ mode: 'auto', single: '', range: [] });
  const [autonomy, setAutonomy] = useState<AutonomyLevel>('interactive');

  // ── Replan modal state ──────────────────────────────────────────────────────
  // Moved over from the deleted floating overlay. v1 simplification: the replan
  // composer prefills the run's goal + autonomy, but the model_range defaults to
  // `auto` (every enabled pair) rather than being reverse-rebuilt from the run's
  // fleet_members snapshot. The user can narrow it in the modal.
  const [replanOpen, setReplanOpen] = useState(false);
  const [replanGoal, setReplanGoal] = useState('');
  const [replanModelRange, setReplanModelRange] = useState<ComposerModelRange>({
    mode: 'auto',
    single: '',
    range: [],
  });
  const [replanAutonomy, setReplanAutonomy] = useState<AutonomyLevel>('interactive');
  const [replanSubmitting, setReplanSubmitting] = useState(false);

  const conversationId = orchestration?.conversationId;

  const handleStart = useCallback(
    async (goal: string) => {
      if (!goal || submitting || conversationId == null) return;
      if (!hasModels) {
        message.warning(t('orchestrator.composer.noModels'));
        return;
      }
      const wireRange = buildModelRange({ mode: modelRange.mode, single: modelRange.single, range: modelRange.range });
      if (!wireRange) {
        message.warning(t('orchestrator.composer.modelRequired'));
        return;
      }

      setSubmitting(true);
      try {
        const body: TCreateAdhocRun = {
          goal,
          model_range: wireRange,
          autonomy,
          lead_conv_id: conversationId,
        };
        await ipcBridge.orchestrator.runs.createAdhoc.invoke(body);
        setIntent('');
        // Do NOT set runId here — the backend persisted the run, linked it to this
        // conversation, and broadcasts `conversation.listChanged`; the conversation
        // refetch lights up `runId` via useConversationRun (F1/F3 链路).
        message.success(
          t('conversation.orchestration.startSuccess', { defaultValue: '已发起编排，正在规划…' })
        );
      } catch (e) {
        const backendMsg = isBackendHttpError(e) && e.backendMessage ? e.backendMessage : '';
        message.error(t('orchestrator.composer.createError', { error: backendMsg || String(e) }));
      } finally {
        setSubmitting(false);
      }
    },
    [submitting, conversationId, hasModels, buildModelRange, modelRange, autonomy, message, t]
  );

  const openReplan = useCallback(() => {
    const goal = orchestration?.detail?.run.goal ?? '';
    setReplanGoal(goal);
    setReplanModelRange({ mode: 'auto', single: '', range: [] });
    setReplanAutonomy(orchestration?.detail?.run.autonomy === 'supervised' ? 'supervised' : 'interactive');
    setReplanOpen(true);
  }, [orchestration?.detail?.run.goal, orchestration?.detail?.run.autonomy]);

  const submitReplan = useCallback(
    async (goal: string) => {
      const runId = orchestration?.runId;
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
        await orchestration?.refetch();
        setReplanOpen(false);
      } catch (e) {
        message.error(t('orchestrator.composer.replanError', { error: String(e) }));
      } finally {
        setReplanSubmitting(false);
      }
    },
    [orchestration, buildModelRange, replanModelRange, replanAutonomy, message, t]
  );

  // Outside an OrchestrationProvider (e.g. the companion「聊天」tab) — degrade to a
  // neutral empty state instead of throwing.
  if (!orchestration) {
    return (
      <div className='size-full flex flex-col items-center justify-center gap-12px px-24px py-32px text-center'>
        <span className='flex size-48px items-center justify-center rd-14px bg-fill-2 text-t-tertiary'>
          <Workbench theme='outline' size='24' strokeWidth={3} />
        </span>
        <div className='text-13px font-600 text-t-secondary'>
          {t('conversation.orchestration.unavailable', { defaultValue: '此会话不支持智能编排' })}
        </div>
      </div>
    );
  }

  const { runId, detail, leadThinking, refetch, projectTask, returnToMain, projectedTaskId } = orchestration;

  // ── No run — Path-B initiation card ─────────────────────────────────────────
  if (runId == null) {
    return (
      <div className='size-full flex flex-col items-center gap-16px px-16px py-24px'>
        {msgCtx}
        <span className='flex size-52px items-center justify-center rd-16px bg-fill-2 text-primary-6'>
          <Workbench theme='outline' size='26' strokeWidth={3} />
        </span>
        <div className='text-center'>
          <div className='text-15px font-600 leading-tight text-t-primary'>
            {t('conversation.orchestration.startTitle', { defaultValue: '发起智能编排' })}
          </div>
          <div className='mt-6px text-12px leading-18px text-t-tertiary'>
            {t('conversation.orchestration.startSubtitle', {
              defaultValue: '把当前会话交给多个 agent 协作完成，过程可在此实时查看。',
            })}
          </div>
        </div>

        {/* Fluid composer — fills the narrow rail (no 800px clamp). */}
        <OrchestratorComposer
          fluid
          value={intent}
          onChange={setIntent}
          onSubmit={handleStart}
          submitting={submitting}
          placeholder={t('conversation.orchestration.startPlaceholder', {
            defaultValue: '描述你想让 agent 团队完成的目标…',
          })}
          label={t('conversation.orchestration.startLabel', { defaultValue: '发起编排' })}
          showModelRange
          modelRange={modelRange}
          onModelRangeChange={setModelRange}
          showAutonomy
          autonomy={autonomy}
          onAutonomyChange={setAutonomy}
        />
      </div>
    );
  }

  // ── Has run — full orchestration surface (status + controls + canvas) ────────
  const status = detail?.run.status ?? '';
  const statusMeta = STATUS_META[status];
  const statusColor = statusMeta?.color ?? STATUS_FALLBACK_COLOR;
  const statusLabel = statusMeta
    ? t(`orchestrator.run.status.${statusMeta.key}`, { defaultValue: status })
    : t('orchestrator.run.status.unknown', { defaultValue: status });

  return (
    <div className='size-full flex flex-col gap-10px p-12px'>
      {msgCtx}

      {/* Control row — status pill + planning hint + run controls. Allowed to
          wrap (`flex-wrap`) because the rail is narrow and RunControls' button
          group can overflow a single line. */}
      <div className='flex flex-wrap items-center gap-x-8px gap-y-6px shrink-0'>
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
        {/* Compact run controls (approve/pause/resume/cancel/replan). Pushed to
            its own wrap line on a narrow rail via the flex-wrap container. */}
        <div className='ml-auto'>
          <RunControls runId={runId} status={status} refetch={refetch} onReplan={openReplan} />
        </div>
      </div>

      {/* Live canvas — FILLS the remaining rail height (flex-1). The react-flow
          canvas needs a bounded box to lay out; the flex column gives it one. */}
      <div className='flex-1 min-h-0 rd-12px overflow-hidden border border-solid border-[var(--color-border-2)] bg-fill-1'>
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

      {/* When the run is freshly created but the plan hasn't arrived yet (no tasks),
          a quiet hint sits below the canvas' own planning state. */}
      {detail && detail.tasks.length === 0 && (
        <div className='shrink-0 flex items-center gap-6px text-11px leading-16px text-t-tertiary'>
          <Branch theme='outline' size='13' strokeWidth={3} />
          <span>{t('conversation.orchestration.awaitingPlan', { defaultValue: '正在生成任务编排…' })}</span>
        </div>
      )}

      {/* Replan modal — a STANDARD Arco dialog (not a floating window): the
          OrchestratorComposer (fluid) prefilled with the run's goal; model-range
          defaults to auto (v1 simplification — not rebuilt from the fleet snapshot)
          + the autonomy pill from the run. On submit → runs.replan → toast +
          refetch + close. */}
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

export default OrchestrationRailTab;
