/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { Suspense, useCallback, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { FullScreen, Branch, Workbench } from '@icon-park/react';
import { Spin } from '@arco-design/web-react';
import { ipcBridge } from '@/common';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import type { TCreateAdhocRun } from '@/common/types/orchestrator/orchestratorTypes';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import OrchestratorComposer, {
  type AutonomyLevel,
  type ComposerModelRange,
} from '@/renderer/pages/orchestrator/OrchestratorComposer';
import { useModelRange } from '@/renderer/pages/orchestrator/useModelRange';
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
 * Two states, both reading {@link useOrchestrationSafe}:
 *  - **has run** (`runId != null`): a status pill (colored from {@link STATUS_META}
 *    via a CSS var), a live「规划中…」hint while the lead agent is still planning,
 *    a height-constrained live {@link DagCanvas} preview (lazy + Suspense), and an
 *    「展开」control that floats the full canvas (F6) via `openCanvas()`.
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

  // Path-B composer state — intent text + model range (defaults to「auto」=
  // every enabled model) + autonomy (defaults to interactive: review the plan).
  const [intent, setIntent] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const [modelRange, setModelRange] = useState<ComposerModelRange>({ mode: 'auto', single: '', range: [] });
  const [autonomy, setAutonomy] = useState<AutonomyLevel>('interactive');

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

  const { runId, detail, leadThinking, projectTask, returnToMain, openCanvas, projectedTaskId } = orchestration;

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

  // ── Has run — live preview + expand ─────────────────────────────────────────
  const status = detail?.run.status ?? '';
  const statusMeta = STATUS_META[status];
  const statusColor = statusMeta?.color ?? STATUS_FALLBACK_COLOR;
  const statusLabel = statusMeta
    ? t(`orchestrator.run.status.${statusMeta.key}`, { defaultValue: status })
    : t('orchestrator.run.status.unknown', { defaultValue: status });

  return (
    <div className='size-full flex flex-col gap-10px p-12px'>
      {msgCtx}

      {/* Header row — status pill + live planning hint. */}
      <div className='flex items-center justify-between gap-8px shrink-0'>
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
            <Spin size={12} />
            <span>{t('conversation.orchestration.planning', { defaultValue: '规划中…' })}</span>
          </span>
        )}
      </div>

      {/* Live canvas preview — height-constrained because the extra-tab content
          renders inside a scrolling FlexFullContainer with no fixed height; the
          react-flow canvas needs a bounded box to lay out. */}
      <div className='shrink-0 h-[clamp(220px,42vh,360px)] min-h-0 rd-12px overflow-hidden border border-solid border-[var(--color-border-2)] bg-fill-1'>
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

      {/* Expand — floats the full canvas (overlay rendered in F6). */}
      <div
        role='button'
        tabIndex={0}
        onClick={openCanvas}
        onKeyDown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            openCanvas();
          }
        }}
        className='shrink-0 inline-flex cursor-pointer select-none items-center justify-center gap-6px rd-8px px-10px py-7px text-12px font-600 text-t-secondary transition-colors hover:bg-fill-2 hover:text-t-primary border border-solid border-[var(--color-border-2)]'
      >
        <FullScreen theme='outline' size='14' strokeWidth={3} />
        <span>{t('conversation.orchestration.expand', { defaultValue: '展开画布' })}</span>
      </div>

      {/* When the run is freshly created but the plan hasn't arrived yet (no tasks),
          a quiet hint sits below the canvas' own planning state. */}
      {detail && detail.tasks.length === 0 && (
        <div className='shrink-0 flex items-center gap-6px text-11px leading-16px text-t-tertiary'>
          <Branch theme='outline' size='13' strokeWidth={3} />
          <span>{t('conversation.orchestration.awaitingPlan', { defaultValue: '正在生成任务编排…' })}</span>
        </div>
      )}
    </div>
  );
};

export default OrchestrationRailTab;
