import { Button, Dropdown, Input, Menu, Select, Spin } from '@arco-design/web-react';
import { Brain, CheckOne, Comment, Down, Help, Left } from '@icon-park/react';
import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ipcBridge } from '@/common';
import type { TChatConversation } from '@/common/config/storage';
import { tryParseEntityId, type ExecutionParticipantId } from '@/common/types/ids';
import type {
  TConfigureExecutionStep,
  TExecutionModelRef,
  TExecutionParticipant,
  TExecutionStep,
  TExecutionStepProfile,
} from '@/common/types/agentExecution/agentExecutionTypes';
import { latestAttemptForStep } from '@/common/types/agentExecution/agentExecutionTypes';
import RouteErrorBoundary from '@/renderer/components/layout/RouteErrorBoundary';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import type { OpenStepPayload } from './DagCanvas';
import { useExecution } from './ExecutionContext';
import { participantShortLabel } from './participantLabel';
import ReadOnlyConversationView from './ReadOnlyConversationView';
import StepConfigBar from './StepConfigBar';
import styles from './projectedAttemptView.module.css';
import { refreshOnVersionConflict } from './refreshOnVersionConflict';
import { canSteerExecutionAttempt, isTerminalExecutionStatus } from './executionStatusMeta';
import { useExecutionLive } from './useExecutionLive';
import ExecutionStopControl from './ExecutionStopControl';
import ExecutionStepStopControl from './ExecutionStepStopControl';
import { attemptRecoveryReview } from './attemptRecoveryReview';
import { selectProjectedAttempt } from './projectedAttemptSelection';
import TaskActivityDetails from './TaskActivityDetails';
import ExecutionTaskRestartControl from './ExecutionTaskRestartControl';

type ProjectedAttemptViewProps = { payload: OpenStepPayload };
type StepConfigDraft = Pick<TConfigureExecutionStep, 'model' | 'preset_prompt'>;
const TOAST_CLASS = 'nomifun-message-passthrough';
const TOAST_OK_MS = 1500;
const TOAST_ERROR_MS = 2500;

function participantSupportsProfile(participant: TExecutionParticipant, profile: TExecutionStepProfile | null): boolean {
  if (!profile) return true;
  const allowedKinds = participant.constraints?.allowed_profile_kinds;
  if (allowedKinds && !allowedKinds.includes(profile.kind)) return false;
  if (profile.needs_vision && !participant.capability?.modalities.includes('vision')) return false;
  if (profile.kind === 'tool' && participant.capability?.tools !== true) return false;
  if (profile.needs_web_search && participant.capability?.web_search !== true) return false;
  return true;
}

const ProjectedAttemptView: React.FC<ProjectedAttemptViewProps> = ({ payload }) => {
  const { t } = useTranslation();
  const { returnToMain, executionId: latestExecutionId, detail: latestDetail, refetch: refreshLatest, projectStep } = useExecution();
  const matchesLatest = latestExecutionId === payload.executionId && latestDetail?.execution.execution_id === payload.executionId;
  const selectedExecution = useExecutionLive(matchesLatest ? undefined : payload.executionId);
  const detail = matchesLatest ? latestDetail
    : selectedExecution.detail?.execution.execution_id === payload.executionId ? selectedExecution.detail : null;
  const refetch = matchesLatest ? refreshLatest : selectedExecution.refetch;
  const [message, messageContext] = useArcoMessage();
  const [conversation, setConversation] = useState<TChatConversation | null>(null);
  const [loading, setLoading] = useState(false);
  const [stoppedScope, setStoppedScope] = useState<string>();
  const [adopting, setAdopting] = useState(false);
  const [reassigning, setReassigning] = useState(false);
  const [decisionAnswer, setDecisionAnswer] = useState('');
  const [answering, setAnswering] = useState(false);
  const [steerText, setSteerText] = useState('');
  const [steering, setSteering] = useState(false);
  const [recovering, setRecovering] = useState(false);
  const [replacementId, setReplacementId] = useState<string>();

  const step = useMemo(
    () => detail?.steps.find((candidate) => candidate.step_id === payload.step.step_id) ?? payload.step,
    [detail?.steps, payload.step],
  );
  const attempts = detail?.attempts ?? (payload.attempt ? [payload.attempt] : []);
  const historical = payload.attemptId !== undefined;
  const attempt = selectProjectedAttempt(attempts, step.step_id, payload.attemptId);
  const recoveryReview = attemptRecoveryReview(attempt?.runtime_state);
  const nativePaused = recoveryReview?.kind === 'native_pause' && recoveryReview.receiptState === 'paused';
  const conversationId = detail ? attempt?.conversation_id : undefined;
  const participants = detail?.participants ?? payload.participants;
  const assignableParticipants = participants.filter(
    (participant) => participant.retired_in_revision == null && participantSupportsProfile(participant, step.profile),
  );
  const participantId = historical ? attempt?.participant_id : step.assigned_participant_id;
  const currentParticipant = participantId
    ? participants.find((participant) => participant.participant_id === participantId)
    : payload.participant;

  useEffect(() => {
    setDecisionAnswer('');
    setSteerText('');
    setReplacementId(undefined);
  }, [attempt?.attempt_id]);
  const automationControl = detail?.automation_control;
  const canConfigure = !historical && detail !== null && !automationControl && step.kind === 'agent' && step.status === 'pending';
  const attemptActive = attempt ? ['queued', 'running', 'waiting_input'].includes(attempt.status) : false;
  const canSteer = !historical && detail !== null && canSteerExecutionAttempt(attempt?.status, detail.execution.status);
  const executionCancelled = detail?.execution.status === 'cancelled';
  const taskScope = `${payload.executionId}:${step.step_id}:${attempt?.attempt_id ?? 'unstarted'}`;
  const canRestart = !historical && detail !== null && !automationControl && !executionCancelled
    && step.kind === 'agent' && step.superseded_in_revision == null && step.status !== 'cancelled'
    && attempt?.status !== 'cancelled' && stoppedScope !== taskScope;
  const canAdopt =
    !historical && conversationId != null && Boolean(detail?.execution) && !automationControl && !executionCancelled && step.status !== 'cancelled' && !attemptActive;
  const canReconcile = !historical && detail !== null && !automationControl && !isTerminalExecutionStatus(detail.execution.status)
    && step.superseded_in_revision == null && step.status !== 'cancelled';
  const replacements = (detail?.steps ?? []).filter(candidate => candidate.step_id !== step.step_id
    && candidate.kind === 'agent' && candidate.status === 'completed' && candidate.superseded_in_revision == null);

  const projectReplacementStep = useCallback(
    (replacement: TExecutionStep) => {
      projectStep({
        ...payload,
        projectionKey: payload.projectionKey ?? payload.step.step_id,
        step: replacement,
        participant: participants.find(
          (participant) => participant.participant_id === replacement.assigned_participant_id,
        ),
        participants,
        attempt: undefined,
        refetch,
      });
    },
    [participants, payload, projectStep, refetch],
  );

  const applyStepConfig = useCallback(
    async (patch: StepConfigDraft) => {
      try {
        const replacement = await ipcBridge.agentExecution.configure.invoke({
          execution_id: payload.executionId,
          step_id: step.step_id,
          updates: {
            model: patch.model,
            preset_prompt: patch.preset_prompt,
            expected_execution_version: detail?.execution.version ?? 0,
            expected_step_version: step.version,
          },
        });
        projectReplacementStep(replacement);
        await refetch();
      } catch (error) {
        await refreshOnVersionConflict(error, refetch);
        throw error;
      }
    },
    [
      detail?.execution.version,
      payload.executionId,
      projectReplacementStep,
      refetch,
      step.step_id,
      step.version,
    ],
  );

  const applyModel = useCallback((model: TExecutionModelRef | null) => applyStepConfig({ model }), [applyStepConfig]);
  const applyPreset = useCallback((preset: string) => applyStepConfig({ preset_prompt: preset.trim() || null }), [applyStepConfig]);

  const reassign = async (participantId: ExecutionParticipantId) => {
    if (reassigning || participantId === step.assigned_participant_id) return;
    setReassigning(true);
    try {
      const replacement = await ipcBridge.agentExecution.reassign.invoke({
        execution_id: payload.executionId,
        step_id: step.step_id,
        updates: {
          participant_id: participantId,
          locked: true,
          expected_execution_version: detail?.execution.version ?? 0,
          expected_step_version: step.version,
        },
      });
      message.success({
        content: t('agentExecution.assignment.updated', {
          defaultValue: '协作者已更新',
        }),
        duration: TOAST_OK_MS,
        className: TOAST_CLASS,
      });
      projectReplacementStep(replacement);
      await refetch();
    } catch (error) {
      await refreshOnVersionConflict(error, refetch);
      message.error({
        content: t('agentExecution.assignment.error', {
          defaultValue: '更换协作者失败：{{error}}',
          error: String(error),
        }),
        duration: TOAST_ERROR_MS,
        className: TOAST_CLASS,
      });
    } finally {
      setReassigning(false);
    }
  };

  const participantMenu = (
    <Menu
      selectedKeys={step.assigned_participant_id ? [String(step.assigned_participant_id)] : []}
      onClickMenuItem={(key) => {
        const participantId = tryParseEntityId('execution-participant', key);
        if (participantId != null) void reassign(participantId);
      }}
    >
      {assignableParticipants.map((participant) => (
        <Menu.Item key={participant.participant_id}>
          {participantShortLabel(participant) ?? participant.participant_id}
        </Menu.Item>
      ))}
    </Menu>
  );

  useEffect(() => {
    if (conversationId == null) {
      setConversation(null);
      setLoading(false);
      return;
    }
    let cancelled = false;
    setLoading(true);
    void ipcBridge.conversation.get
      .invoke({ conversation_id: conversationId })
      .then((next) => !cancelled && setConversation(next ?? null))
      .catch((error) => {
        console.error('[ProjectedAttemptView] Failed to load conversation:', error);
        if (!cancelled) setConversation(null);
      })
      .finally(() => !cancelled && setLoading(false));
    return () => {
      cancelled = true;
    };
  }, [conversationId]);

  const reconcile = async (replace: boolean) => {
    if (recovering || !canReconcile || !detail) return;
    const replacement = replacements.find(candidate => candidate.step_id === replacementId);
    if (replace && !replacement) return;
    setRecovering(true);
    try {
      const target = { execution_id: payload.executionId, step_id: step.step_id,
        updates: { expected_execution_version: detail.execution.version, expected_step_version: step.version } };
      if (replacement && replace) {
        await ipcBridge.agentExecution.replaceWithExisting.invoke({ ...target, updates: { ...target.updates,
          replacement_step_id: replacement.step_id, expected_replacement_step_version: replacement.version } });
      } else {
        await ipcBridge.agentExecution.recheck.invoke(target);
      }
      await refetch();
      message.success({ content: t('agentExecution.recovery.accepted'), duration: TOAST_OK_MS, className: TOAST_CLASS });
    } catch (error) {
      await refreshOnVersionConflict(error, refetch);
      message.error({ content: t('agentExecution.recovery.error', { error: String(error) }), duration: TOAST_ERROR_MS, className: TOAST_CLASS });
    } finally {
      setRecovering(false);
    }
  };

  const adopt = async () => {
    if (adopting) return;
    setAdopting(true);
    try {
      await ipcBridge.agentExecution.adopt.invoke({
        execution_id: payload.executionId,
        step_id: step.step_id,
        updates: {
          expected_execution_version: detail?.execution.version ?? 0,
          expected_step_version: step.version,
        },
      });
      await refetch();
      message.success({
        content: t('agentExecution.adopt.ok', {
          defaultValue: '已采用当前结果',
        }),
        duration: TOAST_OK_MS,
        className: TOAST_CLASS,
      });
    } catch (error) {
      await refreshOnVersionConflict(error, refetch);
      message.error({
        content: t('agentExecution.adopt.error', {
          defaultValue: '采用失败：{{error}}',
          error: String(error),
        }),
        duration: TOAST_ERROR_MS,
        className: TOAST_CLASS,
      });
    } finally {
      setAdopting(false);
    }
  };

  const answerDecision = async () => {
    const answer = decisionAnswer.trim();
    if (answering || !answer || !attempt || !detail?.execution || recoveryReview) return;
    setAnswering(true);
    try {
      await ipcBridge.agentExecution.answerDecision.invoke({
        execution_id: payload.executionId,
        step_id: step.step_id,
        attempt_id: attempt.attempt_id,
        updates: {
          answer,
          expected_execution_version: detail.execution.version,
          expected_step_version: step.version,
          expected_attempt_version: attempt.version,
        },
      });
      setDecisionAnswer('');
      await refetch();
      message.success({
        content: t('agentExecution.question.ok', {
          defaultValue: '决定已提交',
        }),
        duration: TOAST_OK_MS,
        className: TOAST_CLASS,
      });
    } catch (error) {
      await refreshOnVersionConflict(error, refetch);
      message.error({
        content: t('agentExecution.question.error', {
          defaultValue: '提交决定失败：{{error}}',
          error: String(error),
        }),
        duration: TOAST_ERROR_MS,
        className: TOAST_CLASS,
      });
    } finally {
      setAnswering(false);
    }
  };

  const steer = async () => {
    const text = steerText.trim();
    if (steering || !text || !canSteer || !detail?.execution) return;
    setSteering(true);
    try {
      await ipcBridge.agentExecution.steer.invoke({
        execution_id: payload.executionId,
        step_id: step.step_id,
        updates: {
          text,
          expected_execution_version: detail.execution.version,
          expected_step_version: step.version,
        },
      });
      setSteerText('');
      await refetch();
      message.success({
        content: t('agentExecution.steer.ok'),
        duration: TOAST_OK_MS,
        className: TOAST_CLASS,
      });
    } catch (error) {
      await refreshOnVersionConflict(error, refetch);
      message.error({
        content: t('agentExecution.steer.error', { error: String(error) }),
        duration: TOAST_ERROR_MS,
        className: TOAST_CLASS,
      });
    } finally {
      setSteering(false);
    }
  };

  const activateOnKeyboard = (
    event: React.KeyboardEvent<HTMLElement>,
    action: () => void,
    disabled = false,
  ) => {
    if (disabled || (event.key !== 'Enter' && event.key !== ' ')) return;
    event.preventDefault();
    action();
  };

  return (
    <div className={styles.root}>
      {messageContext}
      <div className={styles.banner}>
        <div className={styles.bannerLead}>
          <span className={styles.bannerBadge}>
            <Comment theme='outline' size='13' />
          </span>
          <span className={styles.bannerEyebrow}>
            {t(historical ? 'agentExecution.projection.history' : 'agentExecution.projection.viewing', {
              defaultValue: '当前任务',
            })}
          </span>
          <span className={styles.bannerTitle}>{step.title}</span>
        </div>
        <div className={styles.bannerActions}>
          {historical && detail && <button type='button' className={styles.action}
            onClick={() => projectStep({ ...payload, attemptId: undefined, projectionKey: step.step_id,
              attempt: latestAttemptForStep(attempts, step.step_id), refetch })}>
            {t('agentExecution.projection.current')}
          </button>}
          {!historical && detail && <ExecutionStepStopControl key={step.step_id} executionId={payload.executionId}
            executionVersion={detail.execution.version} executionStatus={detail.execution.status}
            stepId={step.step_id} stepVersion={step.version} stepStatus={step.status}
            delegatedOnly={automationControl?.root_step_id === step.step_id} refetch={refetch}
            onAccepted={scope => { if (scope !== 'delegated_work') setStoppedScope(taskScope); }} />}
          {!historical && detail && <ExecutionStopControl key={payload.executionId} executionId={payload.executionId}
            executionVersion={detail.execution.version} status={detail.execution.status} refetch={refetch}
            automationControl={automationControl} onAccepted={() => setStoppedScope(taskScope)} />}
          {canConfigure && assignableParticipants.length > 1 && (
            <Dropdown trigger='click' position='br' droplist={participantMenu}>
              <div
                role='button'
                tabIndex={0}
                className={styles.action}
                onKeyDown={(event) => activateOnKeyboard(event, () => event.currentTarget.click())}
              >
                <Brain theme='outline' size='13' />
                <span className='max-w-[140px] truncate'>
                  {participantShortLabel(currentParticipant) ??
                    t('agentExecution.assignment.change', {
                      defaultValue: '更换协作者',
                    })}
                </span>
                <Down theme='outline' size='12' />
              </div>
            </Dropdown>
          )}
          {canAdopt && (
            <div
              role='button'
              tabIndex={0}
              aria-disabled={adopting}
              className={`${styles.action} ${styles.actionAdopt}`}
              onClick={adopting ? undefined : () => void adopt()}
              onKeyDown={(event) => activateOnKeyboard(event, () => void adopt(), adopting)}
            >
              <CheckOne theme='outline' size='13' />
              {t('agentExecution.adopt.button', {
                defaultValue: '采用当前结果',
              })}
            </div>
          )}
          {canRestart && <ExecutionTaskRestartControl executionId={payload.executionId} stepId={step.step_id}
            attemptId={attempt?.attempt_id ?? null} executionVersion={detail.execution.version} stepVersion={step.version} refetch={refetch} />}
          <div
            role='button'
            tabIndex={0}
            className={`${styles.action} ${styles.actionPrimary}`}
            onClick={returnToMain}
            onKeyDown={(event) => activateOnKeyboard(event, returnToMain)}
          >
            <Left theme='outline' size='13' />
            {t('agentExecution.projection.return', {
              defaultValue: '返回主对话',
            })}
          </div>
        </div>
      </div>

      {canSteer && (
        <div className={styles.steerBar}>
          <Input.TextArea
            value={steerText}
            onChange={setSteerText}
            autoSize={{ minRows: 1, maxRows: 3 }}
            placeholder={t('agentExecution.steer.placeholder')}
          />
          <Button
            type='primary'
            size='small'
            loading={steering}
            disabled={!steerText.trim()}
            onClick={() => void steer()}
          >
            {t('agentExecution.steer.submit')}
          </Button>
        </div>
      )}

      {!historical && recoveryReview && (
        <div className={styles.questionBanner} role='status'>
          <span className={styles.questionIcon}><Help theme='filled' size='15' /></span>
          <div className={styles.questionBody}>
            <span className={styles.questionEyebrow}>{t(nativePaused ? 'agentExecution.recovery.pausedTitle' : 'agentExecution.recovery.title')}</span>
            <span className={styles.questionContent}>{t(nativePaused ? 'agentExecution.recovery.pausedDescription' : 'agentExecution.recovery.description')}</span>
            <span className={styles.questionHint}>{t(nativePaused && canRestart ? 'agentExecution.recovery.pausedHint'
              : isTerminalExecutionStatus(detail?.execution.status) ? 'agentExecution.recovery.settledHint' : 'agentExecution.recovery.hint')}</span>
            {canReconcile && <div className={styles.questionActions}>
              <Button size='small' loading={recovering} onClick={() => void reconcile(false)}>{t('agentExecution.recovery.recheck')}</Button>
              {replacements.length > 0 && <>
                <Select aria-label={t('agentExecution.recovery.replacement')} placeholder={t('agentExecution.recovery.replacement')}
                  value={replacementId} onChange={setReplacementId} disabled={recovering}
                  options={replacements.map(candidate => ({ label: candidate.title, value: candidate.step_id }))} />
                <Button size='small' disabled={!replacementId || recovering} onClick={() => void reconcile(true)}>{t('agentExecution.recovery.replace')}</Button>
              </>}
            </div>}
            {recoveryReview.reason && <details className={styles.recoveryDetails}>
              <summary>{t('agentExecution.recovery.details')}</summary>
              <p>{recoveryReview.reason}</p>
            </details>}
          </div>
        </div>
      )}

      {!historical && !recoveryReview && step.status === 'waiting_input' && attempt?.question?.trim() && (
        <div className={styles.questionBanner} role='status'>
          <span className={styles.questionIcon}>
            <Help theme='filled' size='15' />
          </span>
          <div className={styles.questionBody}>
            <span className={styles.questionEyebrow}>
              {t('agentExecution.question.title', {
                defaultValue: '需要你的决定',
              })}
            </span>
            <span className={styles.questionContent}>{attempt.question}</span>
            <span className={styles.questionHint}>
              {t('agentExecution.question.hint', {
                defaultValue: '直接提交你的选择，协作者会据此继续任务。',
              })}
            </span>
            <div className={styles.questionActions}>
              <Input.TextArea
                value={decisionAnswer}
                onChange={setDecisionAnswer}
                autoSize={{ minRows: 1, maxRows: 4 }}
                placeholder={t('agentExecution.question.placeholder', {
                  defaultValue: '输入你的决定…',
                })}
              />
              <Button
                type='primary'
                size='small'
                loading={answering}
                disabled={!decisionAnswer.trim() || !detail?.execution}
                onClick={() => void answerDecision()}
              >
                {t('agentExecution.question.submit', {
                  defaultValue: '提交决定',
                })}
              </Button>
            </div>
          </div>
        </div>
      )}

      {attempt && <TaskActivityDetails key={`${payload.executionId}:${step.step_id}:${attempt.attempt_id}`}
        executionId={payload.executionId} stepId={step.step_id} attemptId={attempt.attempt_id}
        onTaskStopAccepted={() => setStoppedScope(taskScope)} />}
      <div className={styles.body}>
        <RouteErrorBoundary>
          {!detail ? (
            <Spin loading className='flex flex-1 items-center justify-center' />
          ) : canConfigure ? (
            <StepConfigBar
              step={step}
              participant={currentParticipant}
              onApplyModel={applyModel}
              onApplyPreset={applyPreset}
            />
          ) : conversationId == null ? (
            <div className={styles.center}>
              <Comment theme='outline' size='26' />
              <div className={styles.emptyTitle}>
                {t(historical ? 'agentExecution.projection.unavailable' : 'agentExecution.transcript.notStarted', {
                  defaultValue: '协作者尚未开始',
                })}
              </div>
            </div>
          ) : loading || (conversation !== null && conversation.id !== conversationId) ? (
            <Spin loading className='flex flex-1 items-center justify-center' />
          ) : conversation ? (
            <ReadOnlyConversationView conversation={conversation} agent_name={step.title} />
          ) : null}
        </RouteErrorBoundary>
      </div>
    </div>
  );
};

export default ProjectedAttemptView;
