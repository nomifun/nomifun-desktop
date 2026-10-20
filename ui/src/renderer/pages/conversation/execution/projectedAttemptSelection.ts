import type { TExecutionAttempt } from '@/common/types/agentExecution/agentExecutionTypes';
import { latestAttemptForStep } from '@/common/types/agentExecution/agentExecutionTypes';
import type { ExecutionAttemptId, ExecutionStepId } from '@/common/types/ids';
import type { OpenStepPayload } from './DagCanvas';

/** A historical event never silently opens a later retry or another task. */
export function selectProjectedAttempt(
  attempts: TExecutionAttempt[],
  stepId: ExecutionStepId,
  attemptId?: ExecutionAttemptId,
): TExecutionAttempt | undefined {
  return attemptId === undefined
    ? latestAttemptForStep(attempts, stepId)
    : attempts.find(attempt => attempt.step_id === stepId && attempt.attempt_id === attemptId);
}

export function executionProjectionKey(payload: Pick<OpenStepPayload, 'executionId' | 'step' | 'attemptId' | 'projectionKey'>): string {
  return `${payload.executionId}:${payload.attemptId ?? payload.projectionKey ?? payload.step.step_id}`;
}
