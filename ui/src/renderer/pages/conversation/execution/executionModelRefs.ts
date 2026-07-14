import type { TExecutionModelRef } from '@/common/types/agentExecution/agentExecutionTypes';

export interface ModelRefReconciliation {
  retained: TExecutionModelRef[];
  active: TExecutionModelRef[];
  removed: TExecutionModelRef[];
}

const MODEL_REF_SEPARATOR = '\0';

export const modelRefKey = (ref: TExecutionModelRef): string => `${ref.provider_id}${MODEL_REF_SEPARATOR}${ref.model}`;

export const sameModelRefs = (left: TExecutionModelRef[], right: TExecutionModelRef[]): boolean =>
  left.length === right.length && left.every((item, index) => modelRefKey(item) === modelRefKey(right[index]));

export const reconcileModelRefs = (
  refs: TExecutionModelRef[],
  configuredPairs: TExecutionModelRef[],
  availablePairs: TExecutionModelRef[],
): ModelRefReconciliation => {
  const configured = new Set(configuredPairs.map(modelRefKey));
  const available = new Set(availablePairs.map(modelRefKey));
  const seen = new Set<string>();
  const retained: TExecutionModelRef[] = [];
  const active: TExecutionModelRef[] = [];
  const removed: TExecutionModelRef[] = [];

  for (const item of refs) {
    const key = modelRefKey(item);
    if (seen.has(key)) continue;
    seen.add(key);
    if (!configured.has(key)) {
      removed.push(item);
      continue;
    }
    retained.push(item);
    if (available.has(key)) active.push(item);
  }

  return { retained, active, removed };
};
