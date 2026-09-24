/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { SshHostId } from '@/common/types/ids';
import { ipcBridge } from '@/common';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import type { IConversationMcpStatus, IProvider, TChatConversation } from '@/common/config/storage';
import { parseError } from '@/common/utils';
import { uuidv7 } from '@/common/utils';
import type {
  AgentHandoffMode,
  AgentSwitchSelection,
  PreviewAgentSessionSwitchResponse,
} from '@/common/types/agentPlatform';
import { CronJobManager } from '@/renderer/pages/cron';
import { useAgentInfo } from '@/renderer/hooks/agent/useAgentInfo';
import { Message } from '@arco-design/web-react';
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Navigate, useNavigate } from 'react-router-dom';
import ChatLayout, { type ChatLayoutProps } from './ChatLayout';
import ChatSlider from './ChatSlider.tsx';
import { isConversationProcessing } from '@/renderer/pages/conversation/utils/conversationRuntime';
import NomiChat from '../platforms/nomi/NomiChat';
import { useNomiModelSelection } from '../platforms/nomi/useNomiModelSelection';
import CollaborationComposerControl from '@/renderer/components/collaboration/CollaborationComposerControl';
import {
  toAppliedCollaborationTemplate,
  type AppliedCollaborationTemplate,
} from '@/renderer/components/collaboration/collaborationTemplateModel';
import type { CollaborationPolicyValue } from '@/renderer/components/collaboration/CollaborationPolicyControl';
import type { TExecutionModelRef } from '@/common/types/agentExecution/agentExecutionTypes';
import { ExecutionProvider } from '../execution/ExecutionContext';
import ExecutionConversationLayout from '../execution/ExecutionConversationLayout';
import ReadOnlyConversationView from '../execution/ReadOnlyConversationView';
import SshHostStatusPill from './SshHostStatusPill';
import SystemPermissionReminder from './SystemPermissionReminder';
import { useWorkspaceExtraTabs } from '../hooks/useWorkspaceExtraTabs';
import { useExecutionModelPool } from '../execution/useExecutionModelPool';
import { reconcileModelRefs, sameModelRefs } from '../execution/executionModelRefs';
import GuidAgentSelector from '@/renderer/pages/guid/components/GuidAgentSelector';
import { useAgentPresets } from '@/renderer/hooks/agent/useAgentPresets';
import {
  isExecutableAgentPreset,
  saveNomiDefaultModel,
} from '@/renderer/pages/guid/hooks/agentSelectionUtils';
import type { GuidAgentSelection } from '@/renderer/pages/guid/types';
import { refreshConversationCache } from '@/renderer/pages/conversation/utils/conversationCache';
import { CreationComposerContext } from '@/renderer/creation/CreationComposerContext';
import { useCreationDraft } from '@/renderer/creation/useCreationDraft';
import type { CreationMode } from '@/renderer/creation/types';
import {
  filterConversationAgentPresets,
  isConversationAgentTemplate,
} from '@/renderer/components/agent/conversationAgentCatalog';
import AgentSwitchDialog from './AgentSwitchDialog';
import { TEMPLATE_I18N_PATH } from '@/renderer/pages/agentSettings/model';
import { officialConversationTemplateKey } from './conversationAgentIdentity';
import {
  capabilityOf,
  capabilitySupportsTechnicalCapability,
} from '@/common/utils/providerModels';
import {
  protocolSupportsReasoningEffort,
  type SessionReasoningEffort,
} from '@/common/types/reasoningEffort';

/** Check whether a specific skill is mounted on the conversation. */
const hasLoadedSkill = (conversation: TChatConversation | undefined, skillName: string): boolean => {
  const skills = (conversation?.extra as { skills?: string[] } | undefined)?.skills;
  return skills?.includes(skillName) ?? false;
};

/** Host id of an SSH-bound session, or undefined for every other conversation. */
const sshHostIdOf = (conversation: TChatConversation | undefined): SshHostId | undefined =>
  (conversation?.extra as { ssh_host_id?: SshHostId } | undefined)?.ssh_host_id;



type NomiConversation = Extract<TChatConversation, { type: 'nomi' }>;

const CompanionConversationRedirect: React.FC<{ conversationId: NomiConversation['id'] }> = ({ conversationId }) => {
  const [target, setTarget] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    void ipcBridge.companion.listCompanions
      .invoke()
      .then(async (companions) => {
        const sessions = await Promise.all(
          companions.map(async (companion) => ({
            companionId: companion.companion_id,
            session: await ipcBridge.companion.getCompanionSession
              .invoke({ companion_id: companion.companion_id })
              .catch(() => ({ conversation_id: null })),
          }))
        );
        const owner = sessions.find((item) => item.session.conversation_id === conversationId);
        if (!cancelled) {
          setTarget(owner
            ? `/nomi?companion=${encodeURIComponent(owner.companionId)}&mode=cohabit`
            : '/nomi?mode=cohabit');
        }
      })
      .catch(() => {
        if (!cancelled) setTarget('/nomi?mode=cohabit');
      });
    return () => {
      cancelled = true;
    };
  }, [conversationId]);

  return target ? <Navigate replace to={target} /> : <div className='size-full bg-1' />;
};

const NomiConversationLayout: React.FC<{
  conversation: NomiConversation;
  chatLayoutProps: Omit<ChatLayoutProps, 'children' | 'workspaceCollaboration' | 'workspaceExtraTabs'>;
  modelSelection: React.ComponentProps<typeof NomiChat>['modelSelection'];
  agentSelectorNode?: React.ReactNode;
  collaborationControlNode: React.ReactNode;
  currentAgentLabel: string;
  modelSelectionDisabled?: boolean;
  reasoningEffort?: SessionReasoningEffort;
  reasoningEffortUpdating?: boolean;
  onReasoningEffortChange?: (value: SessionReasoningEffort | undefined) => Promise<void> | void;
}> = ({
  conversation,
  chatLayoutProps,
  modelSelection,
  agentSelectorNode,
  collaborationControlNode,
  currentAgentLabel,
  modelSelectionDisabled,
  reasoningEffort,
  reasoningEffortUpdating,
  onReasoningEffortChange,
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
        agentSelectorNode={agentSelectorNode}
        cron_job_id={conversation.cron_job_id}
        loadedSkills={(conversation.extra as { skills?: string[] } | undefined)?.skills}
        loadedMcpStatuses={
          (conversation.extra as { mcp_statuses?: IConversationMcpStatus[] } | undefined)?.mcp_statuses
        }
        agent_name={currentAgentLabel}
        currentAgent={conversation.preset_id
          ? { presetId: conversation.preset_id, label: currentAgentLabel }
          : undefined}
        collaboratorSelectorNode={collaborationControlNode}
        modelSelectionDisabled={modelSelectionDisabled}
        reasoningEffort={reasoningEffort}
        reasoningEffortUpdating={reasoningEffortUpdating}
        onReasoningEffortChange={onReasoningEffortChange}
        isProcessing={isConversationProcessing(conversation)}
        creationTasksEnabled={conversation.agent_snapshot?.enabled_capabilities.includes('creation.media') === true}
      />
    </ExecutionConversationLayout>
  );
};

const NomiConversationPanel: React.FC<{
  conversation: NomiConversation;
  sliderTitle: React.ReactNode;
}> = ({ conversation, sliderTitle }) => {
  const hasPreset = Boolean(conversation.preset_id);
  const navigate = useNavigate();
  const creation = useCreationDraft(conversation.id);
  useEffect(() => ipcBridge.agentPlatform.sessions.onAgentChanged.on((event) => {
    if (String(event.agent_session_id) !== String(conversation.id)) return;
    void refreshConversationCache(conversation.id).catch((error) => {
      console.error('[ChatConversation] Failed to refresh switched Agent:', error);
    });
  }), [conversation.id]);
  const { library: agentLibrary, presets: savedAgentPresets, isLoading: agentsLoading, error: agentsError, refresh: refreshAgents } = useAgentPresets();
  const conversationAgentPresets = useMemo(
    () => filterConversationAgentPresets(savedAgentPresets, agentLibrary?.active_bindings ?? []),
    [agentLibrary?.active_bindings, savedAgentPresets],
  );
  const executableAgentPresets = useMemo(
    () => conversationAgentPresets.filter(isExecutableAgentPreset),
    [conversationAgentPresets],
  );
  const conversationOfficialTemplates = useMemo(
    () => (agentLibrary?.official_templates ?? []).filter(isConversationAgentTemplate),
    [agentLibrary?.official_templates],
  );
  const officialTemplateKey = officialConversationTemplateKey(conversation.extra);
  const frozenAgentSelection = useMemo<GuidAgentSelection>(() =>
    officialTemplateKey
      ? { kind: 'template', templateKey: officialTemplateKey }
      : creation.draft.presetId === conversation.preset_id && creation.draft.selectedAgent
      ? creation.draft.selectedAgent
      : conversation.preset_id
      ? { kind: 'preset', presetId: conversation.preset_id }
      : { kind: 'template', templateKey: 'chat.minimal' },
    [conversation.preset_id, creation.draft.presetId, creation.draft.selectedAgent, officialTemplateKey],
  );
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

  const { t } = useTranslation();
  const frozenSessionConfigHint = t('conversation.chat.frozenSessionConfigHint');
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
  const onSelectModel = useCallback(async (provider: IProvider, modelName: string) => {
    if (modelSwitchingRef.current) return false;
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
      if (
        !protocolSupportsReasoningEffort(capability?.protocol)
        || !capabilitySupportsTechnicalCapability(capability, 'reasoning')
      ) {
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
  }, [conversation.id, t]);

  const modelSelection = useNomiModelSelection({
    initialModel: conversation.model,
    onSelectModel,
  });

  const onReasoningEffortChange = useCallback(async (
    next: SessionReasoningEffort | undefined
  ) => {
    if (reasoningEffortUpdatingRef.current) return;
    if (isConversationProcessing(conversation)) {
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

  const rejectFrozenCollaboratorsChange = useCallback((_next: TExecutionModelRef[]) => {}, []);
  const rejectFrozenTemplateChange = useCallback((_next: AppliedCollaborationTemplate | null) => {}, []);
  const rejectFrozenPolicyChange = useCallback((_next: CollaborationPolicyValue) => {}, []);

  useEffect(() => {
    if (!collaboratorReconciliation || collaboratorReconciliation.removed.length === 0) return;
    if (sameModelRefs(collaborators, collaboratorReconciliation.retained)) return;
    setCollaboratorsState(collaboratorReconciliation.retained);
  }, [collaboratorReconciliation, collaborators]);

  // Existing AgentSessions retain the collaboration facts selected at launch.
  // The unified control remains visible as a read-only summary; changing those
  // facts requires creating a new Session from Guid.
  const collaborationAvailable = Boolean(
    conversation.linked_execution_id
      || conversation.execution_template_id
      || conversation.execution_model_pool?.mode === 'range'
      || conversation.agent_snapshot?.enabled_capabilities.includes('agent.collaboration')
  );
  const collaborationControlNode = collaborationAvailable ? (
    <CollaborationComposerControl
      value={activeCollaborators}
      onChange={rejectFrozenCollaboratorsChange}
      mainModel={mainModelRef}
      selectedTemplate={selectedCollaborationTemplate}
      workDir={conversation.extra?.workspace}
      onTemplateApply={rejectFrozenTemplateChange}
      onTemplateClear={() => rejectFrozenTemplateChange(null)}
      policy={collaborationPolicy}
      onPolicyChange={rejectFrozenPolicyChange}
      runtimeType={conversation.type}
      disabled
      disabledReason={frozenSessionConfigHint}
    />
  ) : null;

  const { info: presetPresetInfo } = useAgentInfo(conversation);
  const [agentSwitch, setAgentSwitch] = useState<{
    selection: GuidAgentSelection;
    preview?: PreviewAgentSessionSwitchResponse;
    mode: AgentHandoffMode;
    loading: boolean;
    applying: boolean;
    error?: string;
    errorCode?: string;
  } | null>(null);
  const switchCurrentConversationAgent = useCallback((selection: GuidAgentSelection) => {
    if (isConversationProcessing(conversation)) {
      Message.warning(t('conversation.chat.agentSwitch.waitForTurn'));
      return;
    }
    if (
      (selection.kind === 'preset'
        && frozenAgentSelection.kind === 'preset'
        && selection.presetId === frozenAgentSelection.presetId)
      || (selection.kind === 'template'
        && frozenAgentSelection.kind === 'template'
        && selection.templateKey === frozenAgentSelection.templateKey)
    ) {
      return;
    }
    const wireSelection: AgentSwitchSelection = selection.kind === 'preset'
      ? { kind: 'preset', preset_id: selection.presetId }
      : { kind: 'template', template_key: selection.templateKey };
    // Keeping this flow mounted is intentional: creation.draft and the normal
    // composer draft remain owned by the current Conversation surface.
    setAgentSwitch({ selection, mode: 'context_only', loading: true, applying: false });
    void ipcBridge.agentPlatform.sessions.previewAgentSwitch.invoke({
      agent_session_id: conversation.id,
      request: { selection: wireSelection },
    }).then((preview) => {
      setAgentSwitch((current) => current && ({
        ...current,
        preview,
        mode: preview.handoff.available ? 'continue_task' : 'context_only',
        loading: false,
        error: undefined,
        errorCode: undefined,
      }));
    }).catch((error) => {
      const errorCode = isBackendHttpError(error) ? error.code : undefined;
      const message = errorCode
        ? t(`conversation.chat.agentSwitch.blockers.${errorCode}`, {
            defaultValue: parseError(error),
          })
        : parseError(error);
      setAgentSwitch((current) => current && ({
        ...current,
        loading: false,
        error: t('conversation.chat.agentSwitch.failed', { error: message }),
        errorCode,
      }));
    });
  }, [conversation, creation.draft, frozenAgentSelection, t]);

  const confirmCurrentConversationAgentSwitch = useCallback(() => {
    if (!agentSwitch?.preview || !agentSwitch.preview.can_apply || agentSwitch.applying) return;
    const wireSelection: AgentSwitchSelection = agentSwitch.selection.kind === 'preset'
      ? { kind: 'preset', preset_id: agentSwitch.selection.presetId }
      : { kind: 'template', template_key: agentSwitch.selection.templateKey };
    setAgentSwitch((current) => current && ({
      ...current,
      applying: true,
      error: undefined,
      errorCode: undefined,
    }));
    void ipcBridge.agentPlatform.sessions.applyAgentSwitch.invoke({
      agent_session_id: conversation.id,
      idempotency_key: uuidv7(),
      request: {
        selection: wireSelection,
        handoff_mode: agentSwitch.mode,
        expected_binding_version: agentSwitch.preview.expected_binding_version,
      },
    }).then(async (result) => {
      await refreshConversationCache(conversation.id);
      setAgentSwitch(null);
      Message.success(t('conversation.chat.agentSwitch.success', {
        agent: agentSwitch.selection.kind === 'template'
          ? t(`agentSettings.template.${TEMPLATE_I18N_PATH[agentSwitch.selection.templateKey]}.name`)
          : result.current_agent_label,
      }));
    }).catch((error) => {
      const errorCode = isBackendHttpError(error) ? error.code : undefined;
      const message = errorCode
        ? t(`conversation.chat.agentSwitch.blockers.${errorCode}`, {
            defaultValue: parseError(error),
          })
        : parseError(error);
      setAgentSwitch((current) => current && ({
        ...current,
        applying: false,
        error: t('conversation.chat.agentSwitch.failed', { error: message }),
        errorCode,
      }));
    });
  }, [agentSwitch, conversation.id, t]);

  const frozenPresetId = conversation.preset_id;
  const resolvePreset = useCallback(async () => {
    if (!frozenPresetId) throw new Error(t('conversation.chat.frozenAgentUnavailable'));
    return frozenPresetId;
  }, [frozenPresetId, t]);
  const frozenCreativeAgent = frozenAgentSelection.kind === 'template'
    && frozenAgentSelection.templateKey === 'creative-studio.default'
    && creation.draft.presetId === frozenPresetId;
  const selectCreationMode = (mode: CreationMode) => {
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
  const currentAgentLabel = officialTemplateKey
    ? t(`agentSettings.template.${TEMPLATE_I18N_PATH[officialTemplateKey]}.name`)
    : (creation.draft.presetId === conversation.preset_id ? creation.draft.agentLabel : undefined)
      ?? presetPresetInfo?.name ?? conversation.agent_snapshot?.preset_name ?? 'Agent';
  const agentSelectorNode = (
    <GuidAgentSelector
      presets={executableAgentPresets}
      officialTemplates={conversationOfficialTemplates}
      selection={frozenAgentSelection}
      selectedLabelOverride={currentAgentLabel}
      isLoading={agentsLoading}
      loadError={agentsError}
      onRetry={refreshAgents}
      disabled={isConversationProcessing(conversation) || agentSwitch?.applying === true}
      disabledReason={t('conversation.chat.agentSwitch.waitForTurn')}
      onSelectPreset={(presetId) => switchCurrentConversationAgent({ kind: 'preset', presetId })}
      onSelectTemplate={(templateKey) => switchCurrentConversationAgent({ kind: 'template', templateKey })}
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
  const sshHostId = sshHostIdOf(conversation);

  const chatLayoutProps = {
    title: conversation.name,
    siderTitle: sliderTitle,
    sider: <ChatSlider conversation={conversation} />,
    headerExtra: (
      <div className='flex items-center gap-8px'>
        <SystemPermissionReminder
          conversationId={conversation.id}
          snapshot={conversation.agent_snapshot}
        />
        {/* An SSH-bound session is indistinguishable from a local one everywhere
            else in the chrome, so the host it drives — and whether the link is
            actually up — leads the header. */}
        {sshHostId ? <SshHostStatusPill conversationId={conversation.id} sshHostId={sshHostId} /> : null}
        {/* The collaboration canvas lives beside the mounted conversation; the
            header keeps the existing capability controls. */}
        {!hideAdvancedControls && <CronJobManager
          conversation_id={conversation.id}
          cron_job_id={conversation.cron_job_id}
          hasCronSkill={hasLoadedSkill(conversation, 'cron')}
        />}
      </div>
    ),
    workspaceEnabled,
    workspacePath: conversation.extra?.workspace,
    isTemporaryWorkspace: (conversation.extra as { is_temporary_workspace?: boolean } | undefined)
      ?.is_temporary_workspace,
    knowledgeEnabled,
    knowledgeWritebackAvailable,
    hideAdvancedControls,
  };

  return (
    <CreationComposerContext.Provider value={{ ...creation, presetId: frozenPresetId, resolvePreset, selectMode: selectCreationMode, exit: exitCreation }}>
      <>
        <AgentSwitchDialog
          visible={agentSwitch !== null}
          preview={agentSwitch?.preview}
          currentAgentLabel={agentSwitch?.preview?.current.preset_id === conversation.preset_id
            ? currentAgentLabel : undefined}
          targetAgentLabel={agentSwitch?.selection.kind === 'template'
            ? t(`agentSettings.template.${TEMPLATE_I18N_PATH[agentSwitch.selection.templateKey]}.name`)
            : undefined}
          loading={agentSwitch?.loading ?? false}
          applying={agentSwitch?.applying ?? false}
          mode={agentSwitch?.mode ?? 'context_only'}
          error={agentSwitch?.error}
          errorCode={agentSwitch?.errorCode}
          onModeChange={(mode) => setAgentSwitch((current) => current && ({ ...current, mode }))}
          onConfirm={confirmCurrentConversationAgentSwitch}
          onCancel={() => setAgentSwitch(null)}
          onRecovery={(code) => {
            setAgentSwitch(null);
            void navigate(code === 'AGENT_SESSION_MODEL_INCOMPATIBLE'
              ? '/models?section=chat'
              : '/agent');
          }}
        />
        <NomiConversationLayout
          conversation={conversation}
          chatLayoutProps={chatLayoutProps}
          modelSelection={modelSelection}
          agentSelectorNode={agentSelectorNode}
          collaborationControlNode={collaborationControlNode}
          currentAgentLabel={currentAgentLabel}
          modelSelectionDisabled={modelSwitching || agentSwitch?.applying === true}
          reasoningEffort={reasoningEffort}
          reasoningEffortUpdating={reasoningEffortUpdating}
          onReasoningEffortChange={onReasoningEffortChange}
        />
      </>
    </CreationComposerContext.Provider>
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
    // Use the shared shell and composer with companion-owned configuration
    // callbacks; model/Agent edits must update the companion across all inputs.
    const isCompanionConversation =
      conversation.extra?.companion_session ||
      Boolean(conversation.extra?.companion_id) ||
      conversation.agent_snapshot?.preset_name === 'companion.default' ||
      conversation.agent_snapshot?.enabled_capabilities.includes('companion') === true;
    if (isCompanionConversation) {
      return <CompanionConversationRedirect conversationId={conversation.id} />;
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
