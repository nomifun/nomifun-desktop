import { Modal, Spin } from '@arco-design/web-react';
import { EveryUser, Help, Loading, Right } from '@icon-park/react';
import React, { Suspense, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ipcBridge } from '@/common';
import { latestAttemptForStep } from '@/common/types/agentExecution/agentExecutionTypes';
import { refreshOnVersionConflict } from './refreshOnVersionConflict';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import ExecutionPlanEditor, { type ExecutionModelPoolSelection } from './ExecutionPlanEditor';
import ExecutionAdjustBox from './ExecutionAdjustBox';
import { ExecutionControls } from './ExecutionControls';
import { useExecution } from './ExecutionContext';
import { EXECUTION_STATUS_META } from './executionStatusMeta';
import { stepStatusMeta } from './nodes/StepNode';
import { useExecutionModelPool } from './useExecutionModelPool';
import styles from './executionTopPanel.module.css';

const DagCanvas = React.lazy(() => import('./DagCanvas'));
const CANVAS_WIDTH_KEY = 'nomifun:execution-canvas-width';
const MIN_WIDTH = 320;
const MAX_WIDTH = 860;
const DEFAULT_WIDTH = 480;

function initialWidth(): number {
  try {
    const persisted = Number(localStorage.getItem(CANVAS_WIDTH_KEY));
    if (Number.isFinite(persisted) && persisted >= MIN_WIDTH && persisted <= MAX_WIDTH) {
      return persisted;
    }
  } catch {
    // localStorage may be unavailable in embedded surfaces.
  }
  return DEFAULT_WIDTH;
}

const ExecutionTopPanel: React.FC = () => {
  const { t } = useTranslation();
  const [message, messageContext] = useArcoMessage();
  const execution = useExecution();
  const { buildModelPool } = useExecutionModelPool();
  const [width, setWidth] = useState(initialWidth);
  const dragState = useRef<{ startX: number; startWidth: number } | null>(null);
  const [replanOpen, setReplanOpen] = useState(false);
  const [replanGoal, setReplanGoal] = useState('');
  const [replanModelPool, setReplanModelPool] = useState<ExecutionModelPoolSelection>({
    mode: 'automatic',
    single: '',
    range: [],
  });
  const [replanSubmitting, setReplanSubmitting] = useState(false);

  const {
    executionId,
    detail,
    leadThinking,
    loading,
    refetch,
    projectStep,
    projectedStepId,
    returnToMain,
    canvasOpen,
    setCanvasOpen,
  } = execution;

  useEffect(() => {
    try {
      localStorage.setItem(CANVAS_WIDTH_KEY, String(width));
    } catch {
      // Ignore unavailable storage.
    }
  }, [width]);

  const onResizeStart = useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      if (event.button !== 0) return;
      event.preventDefault();
      dragState.current = { startX: event.clientX, startWidth: width };
      event.currentTarget.setPointerCapture(event.pointerId);
    },
    [width],
  );

  const onResizeMove = useCallback((event: React.PointerEvent<HTMLDivElement>) => {
    if (!dragState.current) return;
    const next = dragState.current.startWidth + dragState.current.startX - event.clientX;
    setWidth(Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, next)));
  }, []);

  const onResizeEnd = useCallback((event: React.PointerEvent<HTMLDivElement>) => {
    dragState.current = null;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  }, []);

  const openReplan = useCallback(() => {
    setReplanGoal(detail?.execution.goal ?? '');
    setReplanModelPool({ mode: 'automatic', single: '', range: [] });
    setReplanOpen(true);
  }, [detail?.execution.goal]);

  const submitReplan = useCallback(
    async (goal: string) => {
      if (!executionId) return;
      const modelPool = buildModelPool(replanModelPool);
      if (!modelPool) {
        message.warning(
          t('agentExecution.editor.model.required', {
            defaultValue: '请选择可用模型',
          }),
        );
        return;
      }
      setReplanSubmitting(true);
      try {
        await ipcBridge.agentExecution.replan.invoke({
          id: executionId,
          updates: {
            goal: goal.trim(),
            model_pool: modelPool,
            expected_version: detail?.execution.version ?? 0,
          },
        });
        returnToMain();
        await refetch();
        setReplanOpen(false);
        message.success(
          t('agentExecution.controls.replanOk', {
            defaultValue: '协作计划已更新',
          }),
        );
      } catch (error) {
        await refreshOnVersionConflict(error, refetch);
        message.error(
          t('agentExecution.controls.replanError', {
            defaultValue: '更新失败：{{error}}',
            error: String(error),
          }),
        );
      } finally {
        setReplanSubmitting(false);
      }
    },
    [buildModelPool, detail?.execution.version, executionId, message, refetch, replanModelPool, returnToMain, t],
  );

  const latestAttemptByStep = useMemo(
    () => new Map((detail?.steps ?? []).map((step) => [step.id, latestAttemptForStep(detail?.attempts ?? [], step.id)])),
    [detail?.attempts, detail?.steps],
  );

  if (!executionId || !canvasOpen) return null;

  const status = detail?.execution.status ?? '';
  const statusMeta = status ? EXECUTION_STATUS_META[status] : undefined;
  const statusColor = statusMeta?.color ?? 'var(--color-text-3)';
  const steps = (detail?.steps ?? []).filter((step) => step.superseded_in_revision == null);
  const runningCount = steps.filter((step) => step.status === 'running').length;
  const completedCount = steps.filter((step) => step.status === 'completed').length;
  const waitingSteps = steps.filter(
    (step) => step.status === 'waiting_input' && Boolean(latestAttemptByStep.get(step.id)?.question?.trim()),
  );

  return (
    <div className={`${styles.panel} shrink-0 flex flex-col`} style={{ width }}>
      {messageContext}
      <div
        className={styles.resizeHandle}
        role='separator'
        aria-orientation='vertical'
        aria-label={t('agentExecution.panel.resize', {
          defaultValue: '调整协作面板宽度',
        })}
        onPointerDown={onResizeStart}
        onPointerMove={onResizeMove}
        onPointerUp={onResizeEnd}
        onPointerCancel={onResizeEnd}
      />

      <div className={`${styles.header} flex flex-wrap items-center gap-x-10px gap-y-6px`}>
        <button
          type='button'
          className={`${styles.toggle} inline-flex items-center gap-6px cursor-pointer select-none`}
          aria-label={t('agentExecution.panel.collapse', {
            defaultValue: '收起协作任务',
          })}
          onClick={() => setCanvasOpen(false)}
        >
          <Right theme='outline' size='14' strokeWidth={3} />
          <span className='text-13px font-600 text-t-primary'>{t('agentExecution.panel.title', { defaultValue: '协作任务' })}</span>
        </button>

        <span
          className='inline-flex items-center gap-6px rd-full px-9px py-3px text-11px font-600 leading-none'
          style={{
            color: statusColor,
            background: 'color-mix(in srgb, currentColor 12%, transparent)',
          }}
        >
          <span className='size-6px rd-full shrink-0' style={{ background: statusColor }} />
          {t(`agentExecution.status.execution.${status}`, {
            defaultValue: status,
          })}
        </span>

        {leadThinking.active && (
          <span className='inline-flex items-center gap-5px text-11px text-primary-6'>
            <Loading theme='outline' size='12' className='animate-spin' />
            {t('agentExecution.thinking.short', { defaultValue: '规划中…' })}
          </span>
        )}

        <div className='ml-auto'>
          <ExecutionControls
            executionId={executionId}
            executionVersion={detail?.execution.version ?? 0}
            status={status}
            inFlightCount={runningCount}
            refetch={refetch}
            onReplan={openReplan}
          />
        </div>
      </div>

      {steps.length > 0 && (
        <div className={styles.canvasProgress} data-testid='execution-canvas-progress'>
          <div className={styles.canvasProgressHeader}>
            <EveryUser theme='outline' size='14' className={styles.canvasProgressIcon} />
            <span className={styles.canvasProgressTitle}>{t('agentExecution.progress.title', { defaultValue: '协作进度' })}</span>
            <span className={styles.canvasProgressText}>
              {t('agentExecution.progress.summary', {
                defaultValue: '{{done}}/{{total}} 个任务已完成',
                done: completedCount,
                total: steps.length,
              })}
            </span>
          </div>

          {waitingSteps.map((step) => {
            const question = latestAttemptByStep.get(step.id)?.question;
            return (
              <button
                key={step.id}
                type='button'
                className={styles.questionBanner}
                onClick={() => {
                  const participant = step.assigned_participant_id
                    ? detail?.participants.find((item) => item.id === step.assigned_participant_id)
                    : undefined;
                  projectStep({
                    step,
                    participant,
                    participants: detail?.participants ?? [],
                    attempt: latestAttemptByStep.get(step.id),
                    executionId,
                    refetch,
                  });
                }}
              >
                <span className={styles.questionPulse}>
                  <Help theme='filled' size='14' />
                </span>
                <span className={styles.questionText}>
                  {t('agentExecution.progress.question', {
                    defaultValue: '任务「{{title}}」需要你的决定',
                    title: step.title,
                  })}
                  <b className={styles.questionPreview}>{question}</b>
                </span>
              </button>
            );
          })}

          <div className={`${styles.chips} nomi-roster-scroll`}>
            {steps.map((step) => {
              const meta = stepStatusMeta(step.status);
              return (
                <button
                  key={step.id}
                  type='button'
                  className={styles.chip}
                  data-active={projectedStepId === step.id ? 'true' : undefined}
                  title={`${step.title} · ${step.status}`}
                  onClick={() => {
                    const participant = step.assigned_participant_id
                      ? detail?.participants.find((item) => item.id === step.assigned_participant_id)
                      : undefined;
                    projectStep({
                      step,
                      participant,
                      participants: detail?.participants ?? [],
                      attempt: latestAttemptByStep.get(step.id),
                      executionId,
                      refetch,
                    });
                  }}
                >
                  <span className={`${styles.chipDot} ${meta.pulse ? styles.chipDotPulse : ''}`} style={{ background: meta.color }} />
                  <span className={styles.chipTitle}>{step.title}</span>
                </button>
              );
            })}
          </div>
        </div>
      )}

      <div className={`${styles.body} flex-1 min-h-0`}>
        <Suspense fallback={<Spin className='m-auto' />}>
          <DagCanvas
            executionId={executionId}
            detail={detail}
            loading={loading}
            refetch={refetch}
            onOpenStep={projectStep}
            leadThinking={leadThinking}
            activeStepId={projectedStepId}
          />
        </Suspense>
      </div>

      {detail && ['running', 'paused', 'awaiting_approval'].includes(status) && (
        <ExecutionAdjustBox detail={detail} refetch={refetch} onApplied={returnToMain} />
      )}

      <Modal
        title={t('agentExecution.controls.replan', {
          defaultValue: '重新规划',
        })}
        visible={replanOpen}
        footer={null}
        onCancel={() => !replanSubmitting && setReplanOpen(false)}
        maskClosable={!replanSubmitting}
        autoFocus={false}
        unmountOnExit
        style={{ width: 'min(640px, calc(100vw - 32px))' }}
      >
        <ExecutionPlanEditor
          fluid
          value={replanGoal}
          onChange={setReplanGoal}
          onSubmit={submitReplan}
          submitting={replanSubmitting}
          placeholder={t('agentExecution.editor.goalPlaceholder', {
            defaultValue: '描述要重新规划的目标…',
          })}
          label={t('agentExecution.controls.replan', {
            defaultValue: '重新规划',
          })}
          showModelPool
          modelPool={replanModelPool}
          onModelPoolChange={setReplanModelPool}
        />
      </Modal>
    </div>
  );
};

export default ExecutionTopPanel;
