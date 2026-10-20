/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type { IProvider, TChatConversation } from '@/common/config/storage';
import { parseError } from '@/common/utils';
import { CronJobManager } from '@/renderer/pages/cron';
import { useAgentInfo } from '@/renderer/hooks/agent/useAgentInfo';
import { Message } from '@arco-design/web-react';
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Navigate, useNavigate } from 'react-router-dom';
import ChatLayout, { type ChatLayoutProps } from './ChatLayout';
import ChatSlider from './ChatSlider.tsx';
import { isConversationProcessing, isConversationBindingBlocked } from '@/renderer/pages/conversation/utils/conversationRuntime';
import NomiChat from '../platforms/nomi/NomiChat';
import { useNomiModelSelection } from '../platforms/nomi/useNomiModelSelection';
import { ExecutionProvider } from '../execution/ExecutionContext';
import ExecutionConversationLayout from '../execution/ExecutionConversationLayout';
import ReadOnlyConversationView from '../execution/ReadOnlyConversationView';
import SshHostStatusPill from './SshHostStatusPill';
import { useWorkspaceExtraTabs } from '../hooks/useWorkspaceExtraTabs';
import AgentSelector from '@/renderer/components/agent/AgentSelector';
import { useConversationAgentCatalog } from '@/renderer/components/agent/useConversationAgentCatalog';
import {
  saveNomiDefaultModel,
} from '@/renderer/pages/guid/hooks/agentSelectionUtils';
import type { GuidAgentSelection } from '@/renderer/pages/guid/types';
import { refreshConversationCache } from '@/renderer/pages/conversation/utils/conversationCache';
import { CreationComposerContext } from '@/renderer/creation/CreationComposerContext';
import { useCreationDraft } from '@/renderer/creation/useCreationDraft';
import type { CreationMode } from '@/renderer/creation/types';
import { useConversationAgentSwitch } from './useConversationAgentSwitch';
import { officialConversationTemplateKey } from './conversationAgentIdentity';
import {
  capabilityOf,
  capabilitySupportsTechnicalCapability,
} from '@/common/utils/providerModels';
import {
  reasoningEffortsForProtocol,
  type SessionReasoningEffort,
} from '@/common/types/reasoningEffort';
import { conversationDestination } from '@/renderer/utils/routes/conversationDestination';

type NomiConversation = Extract<TChatConversation, { type: 'nomi' }>;

const CompanionConversationRedirect: React.FC<{ conversation: NomiConversation }> = ({ conversation }) => (
  <Navigate replace to={conversationDestination(conversation)} />
);

const NomiConversationLayout: React.FC<{
  conversation: NomiConversation;
  chatLayoutProps: Omit<ChatLayoutProps, 'children' | 'workspaceCollaboration' | 'workspaceExtraTabs'>;
  modelSelection: React.ComponentProps<typeof NomiChat>['modelSelection'];
  agentSelectorNode?: React.ReactNode;
  currentAgentLabel: string;
  submissionDisabled?: boolean;
  onBeforeSubmit?: () => Promise<void>;
  modelSelectionDisabled?: boolean;
  reasoningEffort?: SessionReasoningEffort;
  reasoningEffortUpdating?: boolean;
  onReasoningEffortChange?: (value: SessionReasoningEffort | undefined) => Promise<void> | void;
}> = ({
  conversation,
  chatLayoutProps,
  modelSelection,
  agentSelectorNode,
  currentAgentLabel,
  submissionDisabled,
  onBeforeSubmit,
  modelSelectionDisabled,
  reasoningEffort,
  reasoningEffortUpdating,
  onReasoningEffortChange,
}) => {
  const workspaceExtraTabs = useWorkspaceExtraTabs(conversation);

  return (
    <ExecutionConversationLayout
      {...chatLayoutProps}
      sider={chatLayoutProps.embedded ? null : <ChatSlider conversation={conversation} extraTabs={workspaceExtraTabs} />}
      conversation_id={conversation.id}
      workspaceExtraTabs={workspaceExtraTabs}
    >
      <NomiChat
        conversation={conversation}
        modelSelection={modelSelection}
        agentSelectorNode={agentSelectorNode}
        agent_name={currentAgentLabel}
        modelSelectionDisabled={modelSelectionDisabled}
        reasoningEffort={reasoningEffort}
        reasoningEffortUpdating={reasoningEffortUpdating}
        onReasoningEffortChange={onReasoningEffortChange}
        isProcessing={isConversationProcessing(conversation)}
        creationTasksEnabled={conversation.agent_snapshot?.enabled_capabilities.includes('creation.media') === true}
        creationEnabled={!chatLayoutProps.embedded}
        collaborationEnabled={conversation.agent_snapshot?.enabled_capabilities.includes('agent.collaboration') === true}
        hideSessionCapabilities={chatLayoutProps.embedded}
        submissionDisabled={submissionDisabled}
        onBeforeSubmit={onBeforeSubmit}
      />
    </ExecutionConversationLayout>
  );
};

const NomiConversationPanel: React.FC<{
  conversation: NomiConversation;
  sliderTitle: React.ReactNode;
  embedded?: boolean;
  fixedAgentLabel?: string;
  submissionDisabled?: boolean;
  onBeforeSubmit?: () => Promise<void>;
}> = ({ conversation, sliderTitle, embedded, fixedAgentLabel, submissionDisabled, onBeforeSubmit }) => {
  const hasPreset = Boolean(conversation.preset_id);
  const navigate = useNavigate();
  const creation = useCreationDraft(conversation.id, !embedded);
  useEffect(() => ipcBridge.agentPlatform.sessions.onAgentChanged.on((event) => {
    if (String(event.agent_session_id) !== String(conversation.id)) return;
    void refreshConversationCache(conversation.id).catch((error) => {
      console.error('[ChatConversation] Failed to refresh switched Agent:', error);
    });
  }), [conversation.id]);
  const { presets: executableAgentPresets, officialTemplates: conversationOfficialTemplates, isLoading: agentsLoading, error: agentsError, refresh: refreshAgents } = useConversationAgentCatalog();
  const officialTemplateKey = officialConversationTemplateKey(conversation.extra);
  const frozenAgentSelection = useMemo<GuidAgentSelection>(() =>
    officialTemplateKey
      ? { kind: 'template', templateKey: officialTemplateKey }
      : conversation.preset_id
      ? { kind: 'preset', presetId: conversation.preset_id }
      : { kind: 'template', templateKey: 'chat.minimal' },
    [conversation.preset_id, officialTemplateKey],
  );
  const { t } = useTranslation();
  const [modelSwitching, setModelSwitching] = useState(false);
  const [reasoningEffort, setReasoningEffort] = useState<SessionReasoningEffort | undefined>(
    conversation.reasoning_effort
  );
  const [reasoningEffortUpdating, setReasoningEffortUpdating] = useState(false);
  const reasoningEffortUpdatingRef = useRef(false);
  useEffect(() => {
    setReasoningEffort(conversation.reasoning_effort);
  }, [conversation.reasoning_effort]);
  const modelSwitchingRef = useRef(false);
  const agentSwitchingRef = useRef(false);
  const onSelectModel = useCallback(async (provider: IProvider, modelName: string) => {
    if (isConversationBindingBlocked(conversation)) return false;
    if (modelSwitchingRef.current || agentSwitchingRef.current || reasoningEffortUpdatingRef.current) return false;
    modelSwitchingRef.current = true;
    setModelSwitching(true);
    try {
      const switched = await ipcBridge.conversation.switchModel.invoke({
        conversation_id: conversation.id,
        provider_id: provider.id,
        model: modelName,
      });
      if (!switched) return false;
      await saveNomiDefaultModel(provider.id, modelName);
      const capability = capabilityOf(provider, modelName, 'chat');
      const options = capabilitySupportsTechnicalCapability(capability, 'reasoning')
        ? reasoningEffortsForProtocol(capability?.protocol)
        : [];
      if (reasoningEffort !== undefined && !options.includes(reasoningEffort)) {
        setReasoningEffort(undefined);
      }
      void refreshConversationCache(conversation.id).catch((error) => {
        console.error('[ChatConversation] Failed to refresh switched model:', error);
      });
      Message.success(t('agent.model.switchSuccess'));
      return true;
    } catch (error) {
      console.error('[ChatConversation] Failed to switch model:', error);
      Message.error(`${t('agent.model.switchFailed')}: ${parseError(error)}`);
      return false;
    } finally {
      modelSwitchingRef.current = false;
      setModelSwitching(false);
    }
  }, [conversation, reasoningEffort, t]);

  const modelSelection = useNomiModelSelection({
    initialModel: conversation.model,
    onSelectModel,
  });

  const onReasoningEffortChange = useCallback(async (
    next: SessionReasoningEffort | undefined
  ) => {
    if (reasoningEffortUpdatingRef.current || agentSwitchingRef.current || modelSwitchingRef.current) return;
    if (isConversationBindingBlocked(conversation)) {
      Message.warning(t('conversation.chat.modelSwitchAfterTurn'));
      return;
    }
    const previous = reasoningEffort;
    reasoningEffortUpdatingRef.current = true;
    setReasoningEffortUpdating(true);
    setReasoningEffort(next);
    try {
      const updated = await ipcBridge.agentPlatform.sessions.updateReasoning.invoke({
        agent_session_id: conversation.id,
        reasoning_effort: next,
      });
      setReasoningEffort(updated.reasoning_effort);
      await refreshConversationCache(conversation.id);
    } catch (error) {
      setReasoningEffort(previous);
      console.error('[ChatConversation] Failed to update reasoning effort:', error);
      Message.error(`${t('conversation.reasoningEffort.updateFailed')}: ${parseError(error)}`);
    } finally {
      reasoningEffortUpdatingRef.current = false;
      setReasoningEffortUpdating(false);
    }
  }, [conversation, reasoningEffort, t]);

  const { info: presetPresetInfo } = useAgentInfo(conversation);
  const { switching: agentSwitching, switchAgent: switchCurrentConversationAgent } = useConversationAgentSwitch({
    sessionId: conversation.id,
    currentSelection: frozenAgentSelection,
    processing: isConversationBindingBlocked(conversation),
    switchingRef: agentSwitchingRef,
    configurationChangingRef: modelSwitchingRef,
    reasoningUpdatingRef: reasoningEffortUpdatingRef,
  });
  // The composer and creation.draft remain mounted throughout an Agent switch.

  const frozenPresetId = conversation.preset_id;
  const resolvePreset = useCallback(async () => {
    if (!frozenPresetId) throw new Error(t('conversation.chat.frozenAgentUnavailable'));
    return frozenPresetId;
  }, [frozenPresetId, t]);
  const frozenCreativeAgent = officialTemplateKey === 'creative-studio.default';
  const selectCreationMode = (mode: CreationMode) => {
    if (embedded) return;
    if (frozenCreativeAgent) {
      creation.setMode(mode);
      return;
    }
    void navigate(`/guid?creation=${mode}`, {
      state: {
        resetSessionOptions: true,
        selectedAgentTemplateKey: 'creative-studio.default',
        ...(conversation.extra?.workspace ? { workspace: conversation.extra.workspace } : {}),
      },
    });
  };
  const exitCreation = () => creation.setMode(null);
  const currentAgentLabel = fixedAgentLabel ?? presetPresetInfo?.name ?? 'Agent';
  const agentSelectorNode = fixedAgentLabel ? <span data-testid='fixed-app-agent'>{fixedAgentLabel}</span> : (
    <AgentSelector
      presets={executableAgentPresets}
      officialTemplates={conversationOfficialTemplates}
      selection={frozenAgentSelection}
      selectedLabelOverride={currentAgentLabel}
      isLoading={agentsLoading}
      loadError={agentsError}
      onRetry={refreshAgents}
      disabled={isConversationBindingBlocked(conversation) || modelSwitching || reasoningEffortUpdating}
      disabledReason={t('conversation.chat.agentSwitch.waitForTurn')}
      switching={agentSwitching}
      switchingLabel={t('conversation.chat.agentSwitch.loading')}
      onSelectPreset={(presetId) => void switchCurrentConversationAgent({ kind: 'preset', presetId })}
      onSelectTemplate={(templateKey) => void switchCurrentConversationAgent({ kind: 'template', templateKey })}
    />
  );
  const presetResourceKinds = new Set(
    conversation.agent_snapshot?.required_resource_kinds ?? []
  );
  const workspaceEnabled =
    Boolean(conversation.extra?.workspace) &&
    (!hasPreset || presetResourceKinds.has('workspace'));
  const knowledgeEnabled =
    !hasPreset || presetResourceKinds.has('knowledge_base');
  const knowledgeActions = conversation.agent_snapshot
    ?.enabled_capability_actions?.knowledge ?? [];
  const knowledgeWritebackAvailable = knowledgeActions.some((action) =>
    action === 'knowledge/write' || action === 'knowledge/autogen'
  );
  const hideAdvancedControls = hasPreset &&
    (conversation.agent_snapshot?.enabled_capabilities.length ?? 0) === 0;

  const chatLayoutProps = {
    embedded,
    title: conversation.name,
    siderTitle: sliderTitle,
    sider: <ChatSlider conversation={conversation} />,
    headerExtra: (
      <div className='flex items-center gap-8px'>
        {/* An SSH-bound session is indistinguishable from a local one everywhere
            else in the chrome, so the host it drives — and whether the link is
            actually up — leads the header. */}
        <SshHostStatusPill conversation={conversation} />
        {/* The collaboration canvas lives beside the mounted conversation; the
            header keeps the existing capability controls. */}
        {!hideAdvancedControls && <CronJobManager
          conversation_id={conversation.id}
          cron_job_id={conversation.cron_job_id}
          hasCronSkill={conversation.agent_snapshot?.enabled_capabilities.includes('automation.schedule') === true}
        />}
      </div>
    ),
    workspaceEnabled: embedded ? false : workspaceEnabled,
    workspacePath: conversation.extra?.workspace,
    isTemporaryWorkspace: (conversation.extra as { is_temporary_workspace?: boolean } | undefined)
      ?.is_temporary_workspace,
    knowledgeEnabled,
    knowledgeWritebackAvailable,
    hideAdvancedControls: embedded || hideAdvancedControls,
  };

  return (
    <CreationComposerContext.Provider value={embedded ? null : { ...creation, presetId: frozenPresetId, resolvePreset, selectMode: selectCreationMode, exit: exitCreation }}>
      <NomiConversationLayout
        conversation={conversation}
        chatLayoutProps={chatLayoutProps}
        modelSelection={modelSelection}
        agentSelectorNode={agentSelectorNode}
        currentAgentLabel={currentAgentLabel}
        submissionDisabled={submissionDisabled}
        onBeforeSubmit={onBeforeSubmit}
        modelSelectionDisabled={modelSwitching || agentSwitching}
        reasoningEffort={reasoningEffort}
        reasoningEffortUpdating={reasoningEffortUpdating}
        onReasoningEffortChange={onReasoningEffortChange}
      />
    </CreationComposerContext.Provider>
  );
};

const ChatConversation: React.FC<{
  conversation?: TChatConversation;
  embedded?: boolean;
  fixedAgentLabel?: string;
  submissionDisabled?: boolean;
  onBeforeSubmit?: () => Promise<void>;
}> = ({ conversation, embedded, fixedAgentLabel, submissionDisabled, onBeforeSubmit }) => {
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

  // Retired authoring Sessions retain canonical history only. They cannot
  // reopen the old workflow or expose its execution controls.
  if (conversation?.session_purpose === 'plugin_authoring') {
    return <ChatLayout title={conversation.name} conversation_id={conversation.id} hideAdvancedControls disableRename workspaceEnabled={false} sider={null}>
      <ReadOnlyConversationView conversation={conversation} />
    </ChatLayout>;
  }

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
    // Product ownership comes from the canonical opening projected by the host.
    if (conversation.extra?.companion_id) {
      return <CompanionConversationRedirect conversation={conversation} />;
    }
    return (
      <ExecutionProvider conversation={conversation}>
        <NomiConversationPanel key={conversation.id} conversation={conversation} sliderTitle={sliderTitle} embedded={embedded} fixedAgentLabel={fixedAgentLabel} submissionDisabled={submissionDisabled} onBeforeSubmit={onBeforeSubmit} />
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
