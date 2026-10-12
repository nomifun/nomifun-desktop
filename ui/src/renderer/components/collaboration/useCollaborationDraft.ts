import { useCallback, useEffect, useMemo } from 'react';
import type { TProviderWithModel } from '@/common/config/storage';
import type { TExecutionModelRef, TExecutionModelPool, TDelegationPolicy, TDecisionPolicy } from '@/common/types/agentExecution/agentExecutionTypes';
import type { ConversationId, ExecutionTemplateId } from '@/common/types/ids';
import type { CollaborationPolicyValue } from '@/renderer/components/collaboration/CollaborationPolicyControl';
import type { AppliedCollaborationTemplate } from '@/renderer/components/collaboration/collaborationTemplateModel';
import { buildConversationModelPool } from '@/renderer/components/collaboration/conversationModelPool';
import { useExecutionModelPool } from '@/renderer/pages/conversation/execution/useExecutionModelPool';
import { modelRefKey, reconcileModelRefs, sameModelRefs } from '@/renderer/pages/conversation/execution/executionModelRefs';
import { useGuidDraftState, writeGuidDraftState } from '@/renderer/pages/guid/hooks/useGuidDraftState';

export type CollaborationConfig = {
  execution_model_pool: TExecutionModelPool;
  execution_template_id: ExecutionTemplateId | null;
  delegation_policy: TDelegationPolicy;
  decision_policy: TDecisionPolicy;
};

const defaultPolicy = (): CollaborationPolicyValue => ({ delegationPolicy: 'automatic', decisionPolicy: 'automatic' });

export const collaborationDraftScope = (conversationId: ConversationId) => `conversation:${conversationId}`;

/** Unsent configuration only. Submission admits it through AgentExecution, not Session metadata. */
export function useCollaborationDraft(currentModel: TProviderWithModel | undefined, scope?: string) {
  const [collaborators, setCollaborators] = useGuidDraftState<TExecutionModelRef[]>('collaboration-models', [], scope);
  const [policy, setPolicy] = useGuidDraftState<CollaborationPolicyValue>('collaboration-policy', defaultPolicy, scope);
  const [template, setTemplate] = useGuidDraftState<AppliedCollaborationTemplate | null>('collaboration-template', null, scope);
  const { configuredPairs, allPairs, isLoading } = useExecutionModelPool();
  const mainModel = useMemo<TExecutionModelRef | null>(() => currentModel?.use_model
    ? { provider_id: currentModel.id, model: currentModel.use_model } : null, [currentModel?.id, currentModel?.use_model]);
  const reconciliation = useMemo(() => isLoading ? null : reconcileModelRefs(collaborators, configuredPairs, allPairs),
    [collaborators, configuredPairs, allPairs, isLoading]);
  const activeCollaborators = useMemo(() => (reconciliation?.active ?? []).filter(model => !mainModel || modelRefKey(model) !== modelRefKey(mainModel)),
    [reconciliation, mainModel]);
  // A plan validated for another lead model must be chosen again after a switch.
  const selectedTemplate = template && mainModel && template.models.some(model => modelRefKey(model) === modelRefKey(mainModel)) ? template : null;
  useEffect(() => {
    if (reconciliation && !sameModelRefs(collaborators, reconciliation.retained)) setCollaborators(reconciliation.retained);
  }, [reconciliation, collaborators, setCollaborators]);
  useEffect(() => {
    if (template && mainModel && !selectedTemplate) setTemplate(null);
  }, [template, mainModel, selectedTemplate, setTemplate]);
  const reset = useCallback(() => { setCollaborators([]); setPolicy(defaultPolicy()); setTemplate(null); }, [setCollaborators, setPolicy, setTemplate]);
  const pool = buildConversationModelPool(mainModel, activeCollaborators);
  const config: CollaborationConfig | undefined = pool ? {
    execution_model_pool: pool,
    execution_template_id: selectedTemplate?.execution_template_id ?? null,
    delegation_policy: policy.delegationPolicy,
    decision_policy: policy.decisionPolicy,
  } : undefined;
  const copyToConversation = useCallback((conversationId: ConversationId) => {
    const targetScope = collaborationDraftScope(conversationId);
    writeGuidDraftState('collaboration-models', activeCollaborators, targetScope);
    writeGuidDraftState('collaboration-policy', policy, targetScope);
    writeGuidDraftState('collaboration-template', selectedTemplate, targetScope);
  }, [activeCollaborators, policy, selectedTemplate]);
  return { mainModel, activeCollaborators, setCollaborators, policy, setPolicy, selectedTemplate, setTemplate, config, ready: !isLoading, reset, copyToConversation };
}
