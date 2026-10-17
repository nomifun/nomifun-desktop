import type { TaskPlanSnapshot } from '@/common/protocolBindings/TaskPlanSnapshot';

export function deriveTaskPlan(snapshot: TaskPlanSnapshot | null | undefined) {
  if (!snapshot?.plan || snapshot.plan.steps.length === 0) return null;
  const { steps, explanation, needs_replan } = snapshot.plan;
  return {
    steps,
    explanation,
    needsReplan: needs_replan,
    active: snapshot.turn_status === 'running',
    waiting: snapshot.turn_outcome?.kind === 'waiting' || snapshot.turn_outcome?.kind === 'input_required',
    paused: snapshot.turn_status === 'paused' || snapshot.turn_status === 'reconciliation',
    stopped: snapshot.turn_status === 'cancelled' || snapshot.turn_status === 'interrupted',
    failed: snapshot.turn_status === 'failed',
    done: steps.filter((step) => step.status === 'completed').length,
    blocked: steps.filter((step) => step.status === 'blocked').length,
    total: steps.length,
  };
}
