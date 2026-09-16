import { useCallback, useEffect, useMemo } from 'react';
import type { TProviderWithModel } from '@/common/config/storage';
import type { TExecutionModelRef, TExecutionModelPool, TDelegationPolicy, TDecisionPolicy } from '@/common/types/agentExecution/agentExecutionTypes';
import type { ExecutionTemplateId } from '@/common/types/ids';
import type { CollaborationPolicyValue } from '@/renderer/components/collaboration/CollaborationPolicyControl';
import type { AppliedCollaborationTemplate } from '@/renderer/components/collaboration/collaborationTemplateModel';
import { buildConversationModelPool } from '@/renderer/components/collaboration/conversationModelPool';
import { useExecutionModelPool } from '@/renderer/pages/conversation/execution/useExecutionModelPool';
import { modelRefKey, reconcileModelRefs, sameModelRefs } from '@/renderer/pages/conversation/execution/executionModelRefs';
import { useGuidDraftState } from './useGuidDraftState';

export type GuidCollaborationConfig = {
  execution_model_pool: TExecutionModelPool;
  execution_template_id: ExecutionTemplateId | null;
  delegation_policy: TDelegationPolicy;
  decision_policy: TDecisionPolicy;
};

const defaultPolicy = (): CollaborationPolicyValue => ({ delegationPolicy: 'automatic', decisionPolicy: 'automatic' });

export function useGuidCollaboration(currentModel: TProviderWithModel | undefined) {
  const [collaborators, setCollaborators] = useGuidDraftState<TExecutionModelRef[]>('collaboration-models', []);
  const [policy, setPolicy] = useGuidDraftState<CollaborationPolicyValue>('collaboration-policy', defaultPolicy);
  const [template, setTemplate] = useGuidDraftState<AppliedCollaborationTemplate | null>('collaboration-template', null);
  const { configuredPairs, allPairs, isLoading } = useExecutionModelPool();
  const mainModel = useMemo<TExecutionModelRef | null>(() => currentModel?.use_model
    ? { provider_id: currentModel.id, model: currentModel.use_model } : null, [currentModel?.id, currentModel?.use_model]);
  const reconciliation = useMemo(() => isLoading ? null : reconcileModelRefs(collaborators, configuredPairs, allPairs),
    [collaborators, configuredPairs, allPairs, isLoading]);
  const activeCollaborators = (reconciliation?.active ?? []).filter(model => !mainModel || modelRefKey(model) !== modelRefKey(mainModel));
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
  const config: GuidCollaborationConfig | undefined = pool ? {
    execution_model_pool: pool,
    execution_template_id: selectedTemplate?.execution_template_id ?? null,
    delegation_policy: policy.delegationPolicy,
    decision_policy: policy.decisionPolicy,
  } : undefined;
  return { mainModel, activeCollaborators, setCollaborators, policy, setPolicy, selectedTemplate, setTemplate, config, ready: !isLoading, reset };
}
