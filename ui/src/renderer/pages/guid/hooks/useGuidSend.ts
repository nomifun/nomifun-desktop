/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 * Based on AionUi (https://github.com/iOfficeAI/AionUi)
 */

import { ipcBridge } from '@/common';
import type { TProviderWithModel } from '@/common/config/storage';
import {
  conversationTarget,
  parseConversationId,
  type ConversationId,
} from '@/common/types/ids';
import { uuidv7 } from '@/common/utils';
import { sessionStorageKey } from '@/common/utils/browserStorageKey';
import type { PendingConversation } from '@/renderer/pages/conversation/components/ConversationShell/PendingConversationContext';
import type { AutoWorkDraftValue } from '@/renderer/pages/conversation/components/AutoWorkControl';
import { getConversationCreateErrorMessage } from '@/renderer/pages/conversation/utils/conversationCreateError';
import { seedConversationCache } from '@/renderer/pages/conversation/utils/conversationCache';
import { emitter } from '@/renderer/utils/emitter';
import { Message } from '@arco-design/web-react';
import type { TFunction } from 'i18next';
import { useCallback, useRef } from 'react';
import type { NavigateFunction } from 'react-router-dom';
import type {
  ExecutableAgentPreset,
  GuidAgentSelection,
} from '../types';
import { isAutoWorkEntry, planGuidEntry } from './autoWorkEntry';
import type { OfficialPresetTemplate } from '@/common/types/agentPlatform';
import type { AgentResourceSelection } from '@/common/types/agentPlatform';
import { TEMPLATE_I18N_PATH } from '../../agentSettings/model';
import { officialAgentLaunchError, prepareOfficialAgent } from './officialAgentLaunch';
import type { GuidCollaborationConfig } from './useGuidCollaboration';
import { creationDraftStorageKey, emptyCreationDraft } from '@/renderer/creation/useCreationDraft';
import { prepareCompanionConversation, sendCompanionLaunchMessage } from './companionLaunch';

export type GuidSendDeps = {
  input: string;
  setInput: React.Dispatch<React.SetStateAction<string>>;
  files: string[];
  setFiles: React.Dispatch<React.SetStateAction<string[]>>;
  dir: string;
  setDir: React.Dispatch<React.SetStateAction<string>>;
  setLoading: React.Dispatch<React.SetStateAction<boolean>>;
  loading: boolean;
  selection: GuidAgentSelection;
  selectedPreset: ExecutableAgentPreset | undefined;
  selectedTemplate?: OfficialPresetTemplate;
  current_model: TProviderWithModel | undefined;
  applyAdvancedConfig?: (conversationId: ConversationId) => Promise<void>;
  autoWork: AutoWorkDraftValue;
  /** Whether the selected target may receive the staged workspace resource. */
  workspaceEnabled: boolean;
  /** Stable preset capability/resource resolution must finish before launch. */
  resourceResolutionReady: boolean;
  /** Product-selected resources. The backend derives ownership and operations. */
  resourceSelections: AgentResourceSelection[];
  collaboration?: GuidCollaborationConfig;
  setMentionOpen: React.Dispatch<React.SetStateAction<boolean>>;
  setMentionQuery: React.Dispatch<React.SetStateAction<string | null>>;
  setMentionSelectorOpen: React.Dispatch<React.SetStateAction<boolean>>;
  setMentionActiveIndex: React.Dispatch<React.SetStateAction<number>>;
  navigate: NavigateFunction;
  t: TFunction;
  beginPending?: (payload: PendingConversation) => void;
  endPending?: () => void;
};

export type GuidSendResult = {
  handleSend: () => Promise<void>;
  sendMessageHandler: () => void;
  isButtonDisabled: boolean;
};

/**
 * A collaboration draft owns the first piece of work when it selects more than
 * one model, a saved collaboration plan, or an explicit non-default policy. A
 * single-model automatic policy is the ordinary chat default: it grants later
 * delegation but must not invent an AgentExecution before the lead Agent has
 * handled the user's message.
 */
export const shouldStartGuidCollaboration = (
  collaboration: GuidCollaborationConfig | undefined
): collaboration is GuidCollaborationConfig =>
  Boolean(
    collaboration &&
      (collaboration.execution_model_pool.mode === 'range' ||
        collaboration.execution_template_id !== null ||
        collaboration.delegation_policy !== 'automatic' ||
        collaboration.decision_policy !== 'automatic')
  );

const startGuidCollaboration = async (
  conversationId: ConversationId,
  goal: string,
  workspace: string,
  model: TProviderWithModel,
  collaboration: GuidCollaborationConfig
): Promise<void> => {
  const shared = {
    goal: goal.trim(),
    ...(workspace.trim() ? { work_dir: workspace.trim() } : {}),
    delegation_policy: collaboration.delegation_policy,
    decision_policy: collaboration.decision_policy,
    lead_conversation_id: conversationId,
    lead_model: {
      provider_id: model.id,
      model: model.use_model,
    },
  };

  if (collaboration.execution_template_id !== null) {
    await ipcBridge.agentExecutionTemplate.createExecution.invoke({
      execution_template_id: collaboration.execution_template_id,
      request: shared,
    });
    return;
  }

  await ipcBridge.agentExecution.create.invoke({
    ...shared,
    model_pool: collaboration.execution_model_pool,
  });
};

const discardFailedGuidSession = async (conversationId: ConversationId): Promise<void> => {
  try {
    await ipcBridge.agentPlatform.sessions.delete.invoke({
      agent_session_id: conversationId,
    });
  } catch (cleanupError) {
    // Preserve the admission error shown to the user. Cleanup failures remain
    // diagnostic: the backend deletion saga is the only authority that can
    // decide whether a late execution admission made this Session non-empty.
    console.error('[useGuidSend] Failed to discard incomplete AgentSession:', cleanupError);
  }
};

/** Creates a frozen AgentPreset Session from a workbench Agent selection. */
export const useGuidSend = (deps: GuidSendDeps): GuidSendResult => {
  const {
    input,
    setInput,
    files,
    setFiles,
    setDir,
    setLoading,
    loading,
    selection,
    selectedPreset,
    selectedTemplate,
    current_model,
    applyAdvancedConfig,
    autoWork,
    collaboration,
    dir,
    resourceResolutionReady,
    resourceSelections,
    setMentionOpen,
    setMentionQuery,
    setMentionSelectorOpen,
    setMentionActiveIndex,
    navigate,
    t,
    beginPending,
    endPending,
  } = deps;
  const sendingRef = useRef(false);
  const isCompanion = selection.kind === 'template' && selection.templateKey === 'companion.default';

  const handleSend = useCallback(async () => {
    if (isCompanion) {
      if (!resourceResolutionReady) throw new Error('RESOURCE_SELECTION_REQUIRED');
      const conversation = await prepareCompanionConversation(resourceSelections, current_model);
      await sendCompanionLaunchMessage(conversation.id, input, files);
      seedConversationCache(conversation);
      emitter.emit('chat.history.refresh');
      await navigate(`/conversation/${conversation.id}`);
      return;
    }
    const entryPlan = planGuidEntry(input, autoWork);
    if (!current_model) throw new Error('MODEL_REQUIRED');
    if (!resourceResolutionReady) throw new Error('RESOURCE_SELECTION_REQUIRED');
    const startsCollaboration =
      !entryPlan.autoWorkEntry && shouldStartGuidCollaboration(collaboration);
    if (startsCollaboration && files.length > 0) {
      throw new Error(t('guid.collaboration.attachmentsUnsupported'));
    }
    let conversationId: ConversationId;
    let conversation;

    let launchPreset = selectedPreset;
    if (selection.kind === 'template') {
      if (!selectedTemplate || selectedTemplate.template_key !== selection.templateKey) {
        throw new Error('AGENT_PRESET_REQUIRED');
      }
      try {
        launchPreset = await prepareOfficialAgent(
          selectedTemplate,
          t(`agentSettings.template.${TEMPLATE_I18N_PATH[selection.templateKey]}.name`),
          current_model,
        );
      } catch (error) {
        throw new Error(officialAgentLaunchError(error, t));
      }
    }
    if (
      !launchPreset?.current_stable_revision ||
      (selection.kind === 'preset' && launchPreset.preset_id !== selection.presetId)
    ) {
      throw new Error('AGENT_PRESET_REQUIRED');
    }
    const session = await ipcBridge.agentPlatform.sessions.create.invoke({
      preset_id: launchPreset.preset_id,
      title: entryPlan.conversationName,
      model: {
        provider_id: current_model.id,
        model: current_model.use_model,
      },
      ...(resourceSelections.length > 0 ? { resource_selections: resourceSelections } : {}),
    });
    conversationId = parseConversationId(session.agent_session_id);
    try {
      conversation = await ipcBridge.conversation.get.invoke({
        conversation_id: conversationId,
      });
      if (!conversation?.id) {
        throw new Error(
          'AgentSession was created without a Conversation projection'
        );
      }
      await applyAdvancedConfig?.(conversationId);

      if (startsCollaboration) {
        try {
          await startGuidCollaboration(
            conversationId,
            input,
            conversation.extra?.workspace ?? dir,
            current_model,
            collaboration
          );
        } catch (error) {
          // A lost HTTP response must not discard a Session whose Execution
          // was already committed. Recover through the canonical link before
          // treating admission as failed.
          const recovered = await ipcBridge.conversation.get
            .invoke({ conversation_id: conversationId })
            .catch(() => null);
          if (!recovered?.linked_execution_id) throw error;
          conversation = recovered;
        }
        const linked = await ipcBridge.conversation.get
          .invoke({ conversation_id: conversationId })
          .catch((error) => {
            console.error('[useGuidSend] Collaboration started but link refresh failed:', error);
            return null;
          });
        if (linked) conversation = linked;
      } else if (entryPlan.sendInitialMessage) {
        sessionStorage.setItem(
          sessionStorageKey(
            'initial-message-nomi',
            conversationTarget(conversationId)
          ),
          JSON.stringify({
            conversation_id: conversationId,
            initial_admission_epoch: 0,
            input,
            files: files.length > 0 ? files : undefined,
            idempotency_key: uuidv7(),
          })
        );
      }

      // Retain template identity for the next submit; an internal official
      // preset ID alone is indistinguishable from a personal Agent in the UI.
      if (selection.kind === 'template' && !conversation.extra?.companion_session) {
        try {
          const draftKey = creationDraftStorageKey(conversationId);
          if (sessionStorage.getItem(draftKey) === null) {
            sessionStorage.setItem(draftKey, JSON.stringify({
              ...emptyCreationDraft(), selectedAgent: selection, presetId: launchPreset.preset_id,
            }));
          }
        } catch { /* A created session remains usable when browser storage is unavailable. */ }
      }
    } catch (error) {
      await discardFailedGuidSession(conversationId);
      throw error;
    }
    emitter.emit('chat.history.refresh');
    seedConversationCache(conversation);
    await navigate(`/conversation/${conversationId}`);
  }, [
    isCompanion,
    applyAdvancedConfig,
    autoWork,
    current_model,
    collaboration,
    dir,
    files,
    input,
    navigate,
    selection,
    selectedPreset,
    selectedTemplate,
    resourceResolutionReady,
    resourceSelections,
    t,
  ]);

  const launch = useCallback(() => {
    if (loading || sendingRef.current) return;
    if (!resourceResolutionReady) return;
    if (!current_model && !isCompanion) {
      Message.warning(t('conversation.noModelConfigured'));
      return;
    }
    if (selection.kind === 'template' && selectedTemplate?.template_key !== selection.templateKey) return;
    if (
      selection.kind === 'preset' &&
      (!selectedPreset?.current_stable_revision ||
        selectedPreset.preset_id !== selection.presetId)
    ) {
      Message.warning(
        t('guid.agentPresetRequired', {
          defaultValue: 'Select a saved Agent from Agent Workbench first',
        })
      );
      return;
    }

    sendingRef.current = true;
    setLoading(true);
    beginPending?.({
      input,
      files: files.length > 0 ? files : undefined,
      sendsInitialMessage: !isAutoWorkEntry(autoWork),
    });

    handleSend()
      .then(() => {
        setInput('');
        setMentionOpen(false);
        setMentionQuery(null);
        setMentionSelectorOpen(false);
        setMentionActiveIndex(0);
        setFiles([]);
        setDir('');
      })
      .catch((error) => {
        console.error('Failed to create Guid conversation:', error);
        Message.error(getConversationCreateErrorMessage(error, t));
      })
      .finally(() => {
        sendingRef.current = false;
        setLoading(false);
        endPending?.();
      });
  }, [
    isCompanion,
    autoWork,
    beginPending,
    endPending,
    files,
    handleSend,
    input,
    loading,
    current_model,
    resourceResolutionReady,
    selection,
    selectedPreset,
    selectedTemplate,
    setDir,
    setFiles,
    setInput,
    setLoading,
    setMentionActiveIndex,
    setMentionOpen,
    setMentionQuery,
    setMentionSelectorOpen,
    t,
  ]);

  const hasAgentLaunchTarget = selection.kind === 'template'
    ? Boolean(
        selectedTemplate?.template_key === selection.templateKey &&
          resourceResolutionReady
      )
    : Boolean(
        selectedPreset?.current_stable_revision &&
          selectedPreset.preset_id === selection.presetId &&
          resourceResolutionReady
      );
  const hasLaunchTarget = hasAgentLaunchTarget && (isCompanion || Boolean(current_model));
  const isButtonDisabled = loading || !input.trim() || !hasLaunchTarget;
  const sendMessageHandler = useCallback(() => launch(), [launch]);

  return {
    handleSend,
    sendMessageHandler,
    isButtonDisabled,
  };
};
