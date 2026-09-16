import type { TExecutionModelPool, TExecutionModelRef } from '@/common/types/agentExecution/agentExecutionTypes';

export function buildConversationModelPool(mainRef: TExecutionModelRef | null, collaborators: TExecutionModelRef[]): TExecutionModelPool | null {
  if (!mainRef?.provider_id || !mainRef.model) return null;
  const seen = new Set<string>();
  const models = [mainRef, ...collaborators].filter(candidate => {
    if (!candidate.provider_id || !candidate.model) return false;
    const key = `${candidate.provider_id}\u0000${candidate.model}`;
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
  return models.length === 1 ? { mode: 'single', model: models[0] } : { mode: 'range', models };
}
