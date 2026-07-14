import type {
  TAgentExecutionStatus,
  TExecutionAttemptStatus,
} from '@/common/types/agentExecution/agentExecutionTypes';

/**
 * Steering targets one live Attempt. The aggregate may be waiting for input
 * because a sibling step is blocked while this Attempt is still running.
 */
export function canSteerExecutionAttempt(
  attemptStatus: TExecutionAttemptStatus | undefined,
  executionStatus: TAgentExecutionStatus | undefined,
): boolean {
  return attemptStatus === 'running' && (executionStatus === 'running' || executionStatus === 'waiting_input');
}

export const EXECUTION_STATUS_META: Record<TAgentExecutionStatus, { color: string }> = {
  planning: { color: 'var(--warning)' },
  awaiting_approval: { color: 'rgb(var(--primary-6))' },
  running: { color: 'rgb(var(--primary-6))' },
  paused: { color: 'var(--warning)' },
  waiting_input: { color: 'var(--warning)' },
  completed: { color: 'var(--success)' },
  completed_with_failures: { color: 'var(--warning)' },
  failed: { color: 'var(--danger)' },
  cancelled: { color: 'var(--color-text-3)' },
};
