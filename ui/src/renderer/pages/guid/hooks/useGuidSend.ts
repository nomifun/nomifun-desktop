/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 * Based on AionUi (https://github.com/iOfficeAI/AionUi)
 */

import {
  conversationTarget,
  type ConversationId,
  type ExecutionTemplateId,
  type McpServerId,
} from '@/common/types/ids';
import { sessionStorageKey } from '@/common/utils/browserStorageKey';
import { uuidv7 } from '@/common/utils';
import { ipcBridge } from '@/common';
import type { IMcpServer, TProviderWithModel } from '@/common/config/storage';
import { buildAgentConversationParams } from '@/common/utils/buildAgentConversationParams';
import { toSessionMcpServer } from '@/renderer/hooks/mcp/catalog';
import { emitter } from '@/renderer/utils/emitter';
import { Message } from '@arco-design/web-react';
import { useCallback, useRef } from 'react';
import { type TFunction } from 'i18next';
import type { NavigateFunction } from 'react-router-dom';
import { getConversationCreateErrorMessage } from '@/renderer/pages/conversation/utils/conversationCreateError';
import { seedConversationCache } from '@/renderer/pages/conversation/utils/conversationCache';
import type { PendingConversation } from '@/renderer/pages/conversation/components/ConversationShell/PendingConversationContext';
import { planGuidEntry, isAutoWorkEntry } from './autoWorkEntry';
import type { AutoWorkDraftValue } from '@/renderer/pages/conversation/components/AutoWorkControl';
import type { AvailableAgent, EffectiveAgentInfo } from '../types';
import type {
  TDecisionPolicy,
  TDelegationPolicy,
  TExecutionModelPool,
} from '@/common/types/agentExecution/agentExecutionTypes';

export type GuidSendDeps = {
  // Input state
  input: string;
  setInput: React.Dispatch<React.SetStateAction<string>>;
  files: string[];
  setFiles: React.Dispatch<React.SetStateAction<string[]>>;
  dir: string;
  setDir: React.Dispatch<React.SetStateAction<string>>;
  setLoading: React.Dispatch<React.SetStateAction<boolean>>;
  loading: boolean;

  // Agent state
  selectedAgent: string;
  selectedAgentInfo: AvailableAgent | undefined;

  current_model: TProviderWithModel | undefined;

  // Agent helpers
  findAgentByKey: (key: string) => AvailableAgent | undefined;
  getEffectiveAgentType: (
    agentInfo: { agent_type: string; backend?: string } | undefined,
  ) => EffectiveAgentInfo;
  guidDisabledBuiltinSkills: string[] | undefined;
  guidEnabledSkills: string[] | undefined;
  availableMcpServers: IMcpServer[];
  selectedMcpServerIds: McpServerId[] | undefined;

  /** Applies the Guid page's advanced drafts (knowledge/AutoWork/IDMM) onto the
   * freshly created conversation, before navigation. Never throws. */
  applyAdvancedConfig?: (conversationId: ConversationId) => Promise<void>;

  /** Current AutoWork draft. When enabled with a tag, the entry starts an
   * AutoWork session (no initial message) instead of a normal chat send —
   * sending a first message would race the AutoWork turn and surface
   * "conversation N is already running". */
  autoWork: AutoWorkDraftValue;

  delegationPolicy: TDelegationPolicy;
  executionModelPool?: TExecutionModelPool;
  decisionPolicy: TDecisionPolicy;
  /** Optional reusable collaboration input selected in the composer. It is an
   * entry default only; the created Execution copies it and keeps no live FK. */
  executionTemplateId?: ExecutionTemplateId;

  // Mention state reset
  setMentionOpen: React.Dispatch<React.SetStateAction<boolean>>;
  setMentionQuery: React.Dispatch<React.SetStateAction<string | null>>;
  setMentionSelectorOpen: React.Dispatch<React.SetStateAction<boolean>>;
  setMentionActiveIndex: React.Dispatch<React.SetStateAction<number>>;

  // Navigation
  navigate: NavigateFunction;
  t: TFunction;

  /** Show the instant "creating conversation" loading overlay the moment the
   * user sends, before the create round-trip resolves. Optional so callers
   * outside the conversation shell degrade gracefully. */
  beginPending?: (payload: PendingConversation) => void;
  /** Tear the loading overlay down (on success after navigate, or on failure). */
  endPending?: () => void;
};

export type GuidSendResult = {
  handleSend: () => Promise<void>;
  sendMessageHandler: () => void;
  isButtonDisabled: boolean;
};

/**
 * Hook that manages the send logic for conversation creation.
 */
export const useGuidSend = (deps: GuidSendDeps): GuidSendResult => {
  const {
    input,
    setInput,
    files,
    setFiles,
    dir,
    setDir,
    setLoading,
    loading,
    selectedAgent,
    selectedAgentInfo,
    current_model,
    findAgentByKey,
    getEffectiveAgentType,
    guidDisabledBuiltinSkills,
    guidEnabledSkills,
    availableMcpServers,
    selectedMcpServerIds,
    applyAdvancedConfig,
    autoWork,
    delegationPolicy,
    executionModelPool,
    decisionPolicy,
    executionTemplateId,
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

  const handleSend = useCallback(async () => {
    const isCustomWorkspace = !!dir;
    const finalWorkspace = dir || '';

    // AutoWork entry (switch on + tag) creates the session and lets the backend
    // requirement loop drive it — it must NOT also send a first message, which
    // would start a second turn that races the AutoWork turn and loses with
    // "conversation N is already running".
    const entryPlan = planGuidEntry(input, autoWork);

    const agentInfo = selectedAgentInfo;

    const { agent_type: effectiveAgentType } = getEffectiveAgentType(agentInfo);

    const enabledSkills = guidEnabledSkills?.length ? guidEnabledSkills : undefined;
    const excludeBuiltinSkills = guidDisabledBuiltinSkills;
    const selectedMcpServerIdSet = new Set(selectedMcpServerIds ?? []);
    const selectedUserMcpServerIds = availableMcpServers
      .filter((server) => selectedMcpServerIdSet.has(server.mcp_server_id) && server.builtin !== true)
      .map((server) => server.mcp_server_id);
    const selectedAllSessionMcpServers = availableMcpServers
      .filter((server) => selectedMcpServerIdSet.has(server.mcp_server_id))
      .map((server) => toSessionMcpServer(server));
    const selectedSessionMcpServers = availableMcpServers
      .filter((server) => selectedMcpServerIdSet.has(server.mcp_server_id) && server.builtin === true)
      .map((server) => toSessionMcpServer(server));

    const finalEffectiveAgentType = effectiveAgentType;

    // Nomi is the only provider-backed Agent in the quick-start surface.
    if (selectedAgent === 'nomi') {
      if (!current_model) {
        Message.warning(t('conversation.noModelConfigured'));
        return;
      }

      try {
        const conversation = await ipcBridge.conversation.create.invoke({
          type: 'nomi',
          name: entryPlan.conversationName,
          model: current_model,
          delegation_policy: delegationPolicy,
          execution_model_pool: executionModelPool,
          decision_policy: decisionPolicy,
          execution_template_id: executionTemplateId,
          extra: {
            default_files: files,
            workspace: finalWorkspace,
            custom_workspace: isCustomWorkspace,
            agent_enabled_skills: enabledSkills,
            exclude_auto_inject_skills: excludeBuiltinSkills,
            selected_mcp_server_ids: selectedUserMcpServerIds,
            // Nomi consumes the authoritative session snapshot instead of
            // reloading only user servers from the global MCP repository.
            selected_session_mcp_servers: selectedAllSessionMcpServers,
          },
        });

        if (!conversation || !conversation.id) {
          Message.error(t('conversation.createFailed'));
          return;
        }
        // Push the Guid page's advanced drafts (knowledge/AutoWork/IDMM) onto
        // the new conversation before navigating, so they are live when the
        // conversation page consumes the initial message.
        await applyAdvancedConfig?.(conversation.id);

        emitter.emit('chat.history.refresh');

        const initialMessage = {
          conversation_id: conversation.id,
          initial_admission_epoch: 0,
          input,
          files: files.length > 0 ? files : undefined,
          idempotency_key: uuidv7(),
        };
        if (entryPlan.sendInitialMessage) {
          sessionStorage.setItem(
            sessionStorageKey('initial-message-nomi', conversationTarget(conversation.id)),
            JSON.stringify(initialMessage)
          );
        }

        seedConversationCache(conversation);
        await navigate(`/conversation/${conversation.id}`);
      } catch (error: unknown) {
        console.error('Failed to create Nomi conversation:', error);
        throw error;
      }
      return;
    }

    // Remaining Agent path (custom AgentRegistry rows).
    {
      const resolvedBackend: string | undefined = selectedAgent || finalEffectiveAgentType;
      const resolvedAgentInfo = agentInfo || findAgentByKey(resolvedBackend);

      if (!resolvedAgentInfo) {
        console.warn(`${resolvedBackend} agent not found, but proceeding to let conversation panel handle it.`);
      }
      const agentBackend = resolvedBackend || selectedAgent;
      const agentConversationParams = buildAgentConversationParams({
        backend: agentBackend,
        name: entryPlan.conversationName,
        // For row-scoped rows the backend factory needs the actual catalog
        // id — `backend` collapses to the `custom` slot so it cannot
        // discriminate between rows on its own.
        agent_id: resolvedAgentInfo?.id,
        agent_name: resolvedAgentInfo?.name,
        workspace: finalWorkspace,
        model: current_model!,
        cli_path: resolvedAgentInfo?.cli_path,
        custom_workspace: isCustomWorkspace,
        extra: {
          default_files: files,
          exclude_auto_inject_skills: excludeBuiltinSkills,
          selected_mcp_server_ids: selectedUserMcpServerIds,
          selected_session_mcp_servers: selectedSessionMcpServers,
          ...(enabledSkills ? { agent_enabled_skills: enabledSkills } : {}),
        },
      });

      try {
        const conversation = await ipcBridge.conversation.create.invoke(agentConversationParams);
        if (!conversation || !conversation.id) {
          console.error('Failed to create agent conversation - conversation object is null or missing id');
          return;
        }
        await applyAdvancedConfig?.(conversation.id);

        emitter.emit('chat.history.refresh');

        const initialMessage = {
          conversation_id: conversation.id,
          initial_admission_epoch: 0,
          input,
          files: files.length > 0 ? files : undefined,
          idempotency_key: uuidv7(),
        };
        if (entryPlan.sendInitialMessage) {
          const target = conversationTarget(conversation.id);
          const initialMessageKey = sessionStorageKey('initial-message-nomi', target);
          sessionStorage.setItem(initialMessageKey, JSON.stringify(initialMessage));
        }

        seedConversationCache(conversation);
        await navigate(`/conversation/${conversation.id}`);
      } catch (error: unknown) {
        console.error('Failed to create agent conversation:', error);
        throw error;
      }
    }
  }, [
    input,
    files,
    dir,
    selectedAgent,
    selectedAgentInfo,
    current_model,
    findAgentByKey,
    getEffectiveAgentType,
    guidDisabledBuiltinSkills,
    guidEnabledSkills,
    availableMcpServers,
    selectedMcpServerIds,
    applyAdvancedConfig,
    autoWork,
    delegationPolicy,
    executionModelPool,
    decisionPolicy,
    executionTemplateId,
    navigate,
    t,
  ]);

  const sendMessageHandler = useCallback(() => {
    if (loading || sendingRef.current) return;
    sendingRef.current = true;
    setLoading(true);
    // Instant feedback: switch the content region to a conversation-shaped
    // loading overlay (echoed message + "creating…") the moment the user sends,
    // BEFORE the create round-trip resolves. Captured here because `.then` below
    // clears `input`. AutoWork entries send no first message → different caption.
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
        console.error('Failed to send message:', error);
        Message.error(getConversationCreateErrorMessage(error, t));
      })
      .finally(() => {
        sendingRef.current = false;
        setLoading(false);
        // Tear down the overlay: on success the real conversation page has
        // already been navigated to (deferred one frame inside `end`); on
        // failure we uncover the composer with the input preserved.
        endPending?.();
      });
  }, [
    loading,
    handleSend,
    setLoading,
    setInput,
    setMentionOpen,
    setMentionQuery,
    setMentionSelectorOpen,
    setMentionActiveIndex,
    setFiles,
    setDir,
    t,
    input,
    files,
    autoWork,
    beginPending,
    endPending,
  ]);

  // Calculate button disabled state
  const isButtonDisabled = loading || !input.trim();

  return {
    handleSend,
    sendMessageHandler,
    isButtonDisabled,
  };
};
