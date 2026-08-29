/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { SshHostId } from '@/common/types/ids';
import { ipcBridge } from '@/common';
import type { IConversationMcpStatus, IProvider, TChatConversation, TProviderWithModel } from '@/common/config/storage';
import { CronJobManager } from '@/renderer/pages/cron';
import { usePresetInfo } from '@/renderer/hooks/agent/usePresetInfo';
import { Message } from '@arco-design/web-react';
import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import ChatLayout, { type ChatLayoutProps } from './ChatLayout';
import ChatSlider from './ChatSlider.tsx';
import { saveNomiDefaultModel } from '@/renderer/pages/guid/hooks/agentSelectionUtils';
import { configService } from '@/common/config/configService';
import { useModelsForTask } from '@/renderer/hooks/agent/useModelsForTask';
import { resolveHealModel } from '../platforms/nomi/healConversationModel';
import { isConversationProcessing } from '@/renderer/pages/conversation/utils/conversationRuntime';
import NomiChat from '../platforms/nomi/NomiChat';
import { useNomiModelSelection } from '../platforms/nomi/useNomiModelSelection';
import CompanionChatPanel from '@/renderer/pages/nomi/companion/CompanionChatPanel';
import GuidCollaboratorSelector from '@/renderer/pages/guid/components/GuidCollaboratorSelector';
import {
  toAppliedCollaborationTemplate,
  type AppliedCollaborationTemplate,
} from '@/renderer/components/collaboration/collaborationTemplateModel';
import CollaborationPolicyControl, {
  type CollaborationPolicyValue,
} from '@/renderer/components/collaboration/CollaborationPolicyControl';
import type { TExecutionModelPool, TExecutionModelRef } from '@/common/types/agentExecution/agentExecutionTypes';
import { ExecutionProvider } from '../execution/ExecutionContext';
import ExecutionConversationLayout from '../execution/ExecutionConversationLayout';
import ReadOnlyConversationView from '../execution/ReadOnlyConversationView';
import SshHostStatusPill from './SshHostStatusPill';
import { useWorkspaceExtraTabs } from '../hooks/useWorkspaceExtraTabs';
import { useExecutionModelPool } from '../execution/useExecutionModelPool';
import { reconcileModelRefs, sameModelRefs } from '../execution/executionModelRefs';

/** Check whether a specific skill is mounted on the conversation. */
const hasLoadedSkill = (conversation: TChatConversation | undefined, skillName: string): boolean => {
  const skills = (conversation?.extra as { skills?: string[] } | undefined)?.skills;
  return skills?.includes(skillName) ?? false;
};

/** Host id of an SSH-bound session, or undefined for every other conversation. */
const sshHostIdOf = (conversation: TChatConversation | undefined): SshHostId | undefined =>
  (conversation?.extra as { ssh_host_id?: SshHostId } | undefined)?.ssh_host_id;

const buildConversationModelPool = (
  mainRef: TExecutionModelRef | null,
  collaborators: TExecutionModelRef[],
): TExecutionModelPool | null => {
  if (!mainRef?.provider_id || !mainRef.model) return null;
  const seen = new Set<string>();
  const models = [mainRef, ...collaborators].filter((candidate) => {
    if (!candidate.provider_id || !candidate.model) return false;
    const key = `${candidate.provider_id}\u0000${candidate.model}`;
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
  return models.length === 1 ? { mode: 'single', model: models[0] } : { mode: 'range', models };
};

type NomiConversation = Extract<TChatConversation, { type: 'nomi' }>;

const NomiConversationLayout: React.FC<{
  conversation: NomiConversation;
  chatLayoutProps: Omit<ChatLayoutProps, 'children' | 'workspaceCollaboration' | 'workspaceExtraTabs'>;
  modelSelection: React.ComponentProps<typeof NomiChat>['modelSelection'];
  collaborationControlNode: React.ReactNode;
  presetPresetName?: string;
}> = ({
  conversation,
  chatLayoutProps,
  modelSelection,
  collaborationControlNode,
  presetPresetName,
}) => {
  const workspaceExtraTabs = useWorkspaceExtraTabs(conversation);

  return (
    <ExecutionConversationLayout
      {...chatLayoutProps}
      sider={<ChatSlider conversation={conversation} extraTabs={workspaceExtraTabs} />}
      conversation_id={conversation.id}
      workspaceExtraTabs={workspaceExtraTabs}
    >
      <NomiChat
        conversation_id={conversation.id}
        workspace={conversation.extra.workspace}
        modelSelection={modelSelection}
        cron_job_id={conversation.cron_job_id}
        loadedSkills={(conversation.extra as { skills?: string[] } | undefined)?.skills}
        loadedMcpStatuses={
          (conversation.extra as { mcp_statuses?: IConversationMcpStatus[] } | undefined)?.mcp_statuses
        }
        agent_name={presetPresetName}
        collaboratorSelectorNode={collaborationControlNode}
        isProcessing={isConversationProcessing(conversation)}
      />
    </ExecutionConversationLayout>
  );
};

const NomiConversationPanel: React.FC<{
  conversation: NomiConversation;
  sliderTitle: React.ReactNode;
}> = ({ conversation, sliderTitle }) => {
  const [collaborators, setCollaboratorsState] = useState<TExecutionModelRef[]>(() => {
    const pool = conversation.execution_model_pool;
    return pool?.mode === 'range' ? pool.models.slice(1) : [];
  });
  const [collaborationPolicy, setCollaborationPolicy] = useState<CollaborationPolicyValue>({
    delegationPolicy: conversation.delegation_policy ?? 'automatic',
    decisionPolicy: conversation.decision_policy ?? 'automatic',
  });
  const [selectedCollaborationTemplate, setSelectedCollaborationTemplate] =
    useState<AppliedCollaborationTemplate | null>(null);
  useEffect(() => {
    setCollaborationPolicy({
      delegationPolicy: conversation.delegation_policy ?? 'automatic',
      decisionPolicy: conversation.decision_policy ?? 'automatic',
    });
  }, [conversation.decision_policy, conversation.delegation_policy]);

  const storedExecutionTemplateId = conversation.execution_template_id ?? null;
  useEffect(() => {
    if (!storedExecutionTemplateId) {
      setSelectedCollaborationTemplate(null);
      return;
    }
    let cancelled = false;
    void ipcBridge.agentExecutionTemplate.get
      .invoke({ execution_template_id: storedExecutionTemplateId })
      .then((template) => {
        if (!cancelled) {
          setSelectedCollaborationTemplate(toAppliedCollaborationTemplate(template));
        }
      })
      .catch((error) => {
        console.error('[ChatConversation] Failed to resolve collaboration template:', error);
        if (!cancelled) setSelectedCollaborationTemplate(null);
      });
    return () => {
      cancelled = true;
    };
  }, [storedExecutionTemplateId]);
  const { configuredPairs, allPairs, isLoading: isModelCatalogLoading } = useExecutionModelPool();
  const collaboratorReconciliation = useMemo(
    () => (isModelCatalogLoading ? null : reconcileModelRefs(collaborators, configuredPairs, allPairs)),
    [allPairs, collaborators, configuredPairs, isModelCatalogLoading],
  );
  const activeCollaborators = collaboratorReconciliation?.active ?? [];

  const persistModelPool = useCallback(
    async (mainRef: TExecutionModelRef | null, collabs: TExecutionModelRef[]) => {
      const execution_model_pool = buildConversationModelPool(mainRef, collabs);
      if (!execution_model_pool) return;
      try {
        await ipcBridge.conversation.update.invoke({
          conversation_id: conversation.id,
          updates: { execution_model_pool },
        });
      } catch (err) {
        console.error('[ChatConversation] Failed to persist execution model pool:', err);
      }
    },
    [conversation.id],
  );

  const { t } = useTranslation();
  const onSelectModel = useCallback(
    async (_provider: IProvider, modelName: string) => {
      const selected = {
        ..._provider,
        use_model: modelName,
      } as TProviderWithModel;
      // Kill the running agent on model switch; it will be rebuilt with the
      // new model on the next message.
      await ipcBridge.conversation.stop.invoke({
        conversation_id: conversation.id,
      });
      const execution_model_pool = buildConversationModelPool(
        { provider_id: _provider.id, model: modelName },
        activeCollaborators,
      );
      if (!execution_model_pool) return false;
      const ok = await ipcBridge.conversation.update.invoke({
        conversation_id: conversation.id,
        // The lead model and its collaboration authority are one atomic
        // Conversation preference update; never expose a mixed intermediate
        // state to Gateway delegation.
        updates: { model: selected, execution_model_pool, execution_template_id: null },
      });
      if (ok) {
        setSelectedCollaborationTemplate(null);
        void saveNomiDefaultModel(_provider.id, modelName);
      }
      return Boolean(ok);
    },
    [activeCollaborators, conversation.id],
  );

  const modelSelection = useNomiModelSelection({
    initialModel: conversation.model,
    onSelectModel,
  });

  // Main model reference used by the collaboration selector.
  const mainModelRef = useMemo<TExecutionModelRef | null>(
    () =>
      modelSelection.current_model
        ? {
            provider_id: modelSelection.current_model.id,
            model: modelSelection.current_model.use_model,
          }
        : null,
    [modelSelection.current_model?.id, modelSelection.current_model?.use_model],
  );

  const onCollaboratorsChange = useCallback(
    (next: TExecutionModelRef[]) => {
      setCollaboratorsState(next);
      void persistModelPool(mainModelRef, next);
    },
    [mainModelRef, persistModelPool],
  );

  const persistCollaborationTemplate = useCallback(
    async (next: AppliedCollaborationTemplate | null) => {
      const previous = selectedCollaborationTemplate;
      setSelectedCollaborationTemplate(next);
      try {
        await ipcBridge.conversation.update.invoke({
          conversation_id: conversation.id,
          updates: {
            execution_template_id: next?.execution_template_id ?? null,
          },
        });
      } catch (error) {
        setSelectedCollaborationTemplate(previous);
        console.error('[ChatConversation] Failed to persist collaboration template:', error);
        Message.error(t('common.failed', { defaultValue: '保存协作方案失败' }));
      }
    },
    [conversation.id, selectedCollaborationTemplate, t],
  );

  useEffect(() => {
    if (!collaboratorReconciliation || collaboratorReconciliation.removed.length === 0) return;
    if (sameModelRefs(collaborators, collaboratorReconciliation.retained)) return;
    setCollaboratorsState(collaboratorReconciliation.retained);
    void persistModelPool(mainModelRef, collaboratorReconciliation.retained);
  }, [collaboratorReconciliation, collaborators, mainModelRef, persistModelPool]);

  const onCollaborationPolicyChange = useCallback(
    async (next: CollaborationPolicyValue) => {
      setCollaborationPolicy(next);
      try {
        await ipcBridge.conversation.update.invoke({
          conversation_id: conversation.id,
          updates: {
            delegation_policy: next.delegationPolicy,
            decision_policy: next.decisionPolicy,
          },
        });
      } catch (error) {
        console.error('[ChatConversation] Failed to persist collaboration policy:', error);
      }
    },
    [conversation.id],
  );

  // Conversation collaboration models, reusable plans, and policy share one
  // toolbar entry. Their existing callbacks stay independent so this remains
  // a presentation-only merge.
  const collaborationControlNode = (
    <GuidCollaboratorSelector
      value={activeCollaborators}
      onChange={onCollaboratorsChange}
      mainModel={mainModelRef}
      selectedTemplate={selectedCollaborationTemplate}
      workDir={conversation.extra?.workspace}
      onTemplateApply={(template) => void persistCollaborationTemplate(template)}
      onTemplateClear={() => void persistCollaborationTemplate(null)}
      className='nomi-sendbox-model-btn nomi-sendbox-collaboration-btn'
      triggerLabel={t('collaboration.policy.button', { defaultValue: 'Collaboration' })}
      triggerActive={collaborationPolicy.delegationPolicy !== 'disabled'}
      panelFooter={
        <CollaborationPolicyControl
          runtimeType={conversation.type}
          delegationPolicy={collaborationPolicy.delegationPolicy}
          decisionPolicy={collaborationPolicy.decisionPolicy}
          onChange={onCollaborationPolicyChange}
          embedded
        />
      }
    />
  );

  // Heal against exact enabled Chat capabilities, with no name heuristics.
  // While capability data is unavailable/loading `chatGroups` is empty, so resolveHealModel is a
  // no-op, so a transient error can never trigger a destructive model swap.
  const { groups: healGroups } = useModelsForTask('chat');
  const healPool = useMemo(
    () => ({
      providers: healGroups.map((group) => group.provider),
      getAvailableModels: (p: IProvider) =>
        healGroups.find((group) => group.provider.id === p.id)?.models ?? [],
    }),
    [healGroups],
  );
  const { providers: healProviders, getAvailableModels: healGetAvailable } = healPool;
  useEffect(() => {
    if (!healProviders.length) return;
    const saved = configService.get('nomi.defaultModel');
    const heal = resolveHealModel(
      conversation.model,
      healProviders,
      healGetAvailable,
      saved,
    );
    if (!heal) return;
    void (async () => {
      const selected = {
        ...heal.provider,
        use_model: heal.use_model,
      } as TProviderWithModel;
      const execution_model_pool = buildConversationModelPool(
        { provider_id: heal.provider.id, model: heal.use_model },
        activeCollaborators,
      );
      if (!execution_model_pool) return;
      const ok = await ipcBridge.conversation.update.invoke({
        conversation_id: conversation.id,
        updates: { model: selected, execution_model_pool, execution_template_id: null },
      });
      if (ok) {
        setSelectedCollaborationTemplate(null);
        void saveNomiDefaultModel(heal.provider.id, heal.use_model);
        Message.info(
          t('conversation.chat.modelHealedToDefault', {
            model: heal.use_model,
          }),
        );
      }
    })();
    // Re-evaluate when the conversation or provider list changes.
  }, [
    activeCollaborators,
    conversation.id,
    conversation.model?.id,
    conversation.model?.use_model,
    healProviders,
    healGetAvailable,
    t,
  ]);

  const workspaceEnabled = Boolean(conversation.extra?.workspace);
  const { info: presetPresetInfo } = usePresetInfo(conversation);
  const sshHostId = sshHostIdOf(conversation);

  const chatLayoutProps = {
    title: conversation.name,
    siderTitle: sliderTitle,
    sider: <ChatSlider conversation={conversation} />,
    headerExtra: (
      <div className='flex items-center gap-8px'>
        {/* An SSH-bound session is indistinguishable from a local one everywhere
            else in the chrome, so the host it drives — and whether the link is
            actually up — leads the header. It is also the one control kept on
            mobile (ChatLayout portals headerExtra into the mobile actions slot):
            knowing which machine you are typing at matters more on a phone, not
            less. */}
        {sshHostId ? <SshHostStatusPill conversationId={conversation.id} sshHostId={sshHostId} /> : null}
        {/* The collaboration canvas lives beside the mounted conversation; the
            header keeps the existing capability controls. */}
        <CronJobManager
          conversation_id={conversation.id}
          cron_job_id={conversation.cron_job_id}
          hasCronSkill={hasLoadedSkill(conversation, 'cron')}
        />
      </div>
    ),
    workspaceEnabled,
    workspacePath: conversation.extra?.workspace,
    isTemporaryWorkspace: (conversation.extra as { is_temporary_workspace?: boolean } | undefined)
      ?.is_temporary_workspace,
    backend: 'nomi' as const,
    preset: presetPresetInfo ?? undefined,
  };

  return (
    <NomiConversationLayout
      conversation={conversation}
      chatLayoutProps={chatLayoutProps}
      modelSelection={modelSelection}
      collaborationControlNode={collaborationControlNode}
      presetPresetName={presetPresetInfo?.name}
    />
  );
};

const ChatConversation: React.FC<{
  conversation?: TChatConversation;
}> = ({ conversation }) => {
  const { t } = useTranslation();
  const workspaceEnabled = Boolean(conversation?.extra?.workspace);

  const sliderTitle = useMemo(() => {
    return (
      <div className='flex items-center justify-between'>
        <span className='text-16px font-bold text-t-primary'>{t('conversation.workspace.title')}</span>
      </div>
    );
  }, [t]);

  const workspaceExtraTabs = useWorkspaceExtraTabs(conversation);

  const isRetainedAttemptTranscript = Boolean(
    conversation?.execution_step_id || conversation?.execution_attempt_id,
  );

  // An Attempt Conversation is immutable execution audit data, not a second
  // ordinary chat entry point. Direct/history navigation therefore uses the
  // same read-only projection as the collaboration canvas; decisions, steer,
  // retry and lifecycle changes remain AgentExecution commands.
  if (conversation && isRetainedAttemptTranscript) {
    return (
      <ExecutionProvider conversation={conversation}>
        <ExecutionConversationLayout
          title={conversation.name}
          conversation_id={conversation.id}
          hideAdvancedControls
          disableRename
          siderTitle={sliderTitle}
          sider={<ChatSlider conversation={conversation} extraTabs={workspaceExtraTabs} />}
          workspaceEnabled={Boolean(conversation.extra?.workspace)}
          workspacePath={conversation.extra?.workspace}
          isTemporaryWorkspace={
            (conversation.extra as { is_temporary_workspace?: boolean } | undefined)
              ?.is_temporary_workspace
          }
          workspaceExtraTabs={workspaceExtraTabs}
        >
          <ReadOnlyConversationView
            conversation={conversation}
            agent_name={(conversation.extra as { agent_name?: string } | undefined)?.agent_name}
          />
        </ExecutionConversationLayout>
      </ExecutionProvider>
    );
  }

  if (conversation && conversation.type === 'nomi') {
    // Companion sessions use a fixed workspace and restricted controls.
    // Configuration controls remain limited for companion sessions, while
    // linked execution progress and lifecycle state stay visible.
    if (conversation.extra?.companion_session) {
      return (
        <ExecutionProvider conversation={conversation}>
          <CompanionChatPanel
            key={conversation.id}
            conversation={conversation}
            extraTabs={workspaceExtraTabs}
          />
        </ExecutionProvider>
      );
    }
    return (
      <ExecutionProvider conversation={conversation}>
        <NomiConversationPanel key={conversation.id} conversation={conversation} sliderTitle={sliderTitle} />
      </ExecutionProvider>
    );
  }

  // Every conversation type is handled by an early return above (`nomi`, or a
  // retained Attempt transcript), so only the not-yet-loaded shell remains.
  return (
    <ChatLayout
      title={undefined}
      siderTitle={sliderTitle}
      sider={<ChatSlider conversation={undefined} />}
      workspaceEnabled={workspaceEnabled}
    />
  );
};

export default ChatConversation;
