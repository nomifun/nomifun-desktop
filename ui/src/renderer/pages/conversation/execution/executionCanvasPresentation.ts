/**
 * Turn persisted task prose into a bounded canvas summary. Full output remains
 * available in the step inspector; the graph only needs a clean quick read.
 */
export function summarizeExecutionText(value: string | null | undefined, maxLength = 240): string | undefined {
  if (!value) return undefined;
  const normalized = value
    .replace(/<[^>]+>/g, ' ')
    .replace(/```[\w-]*/g, ' ')
    .replace(/`([^`]+)`/g, '$1')
    .replace(/[*~]{1,3}/g, '')
    .replace(/\[([^\]]+)]\([^)]+\)/g, '$1')
    .replace(/(^|\s)#{1,6}\s+/g, ' ')
    .replace(/(^|\s)[>*+-]\s+/g, ' ')
    .replace(/\|/g, ' · ')
    .replace(/\s+/g, ' ')
    .trim();
  if (!normalized) return undefined;

  const limit = Math.max(2, maxLength);
  if (normalized.length <= limit) return normalized;
  return `${normalized.slice(0, limit - 1).trimEnd()}…`;
}

/** Prefer hover focus, then the projected detail, but never focus a node that
 * disappeared when the planner published a new immutable revision. */
export function resolveExecutionCanvasFocusStepId<T>(
  activeStepIds: ReadonlySet<T>,
  hoveredStepId: T | null | undefined,
  projectedStepId: T | null | undefined,
): T | null {
  if (hoveredStepId != null && activeStepIds.has(hoveredStepId)) return hoveredStepId;
  if (projectedStepId != null && activeStepIds.has(projectedStepId)) return projectedStepId;
  return null;
}

export type ExecutionCanvasRelationState = 'idle' | 'focus' | 'related' | 'muted';

/**
 * A projected/selected step keeps its path accented without washing out the
 * rest of the graph. Muting is reserved for the temporary hover inspection.
 */
export function resolveExecutionCanvasRelationState(
  hasFocusedPath: boolean,
  isFocusStep: boolean,
  isRelatedStep: boolean,
  hasTransientFocus: boolean,
): ExecutionCanvasRelationState {
  if (!hasFocusedPath) return 'idle';
  if (isFocusStep) return 'focus';
  if (isRelatedStep) return 'related';
  return hasTransientFocus ? 'muted' : 'idle';
}
