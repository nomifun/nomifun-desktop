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
  current_model: TProviderWithModel | undefined;
  applyAdvancedConfig?: (conversationId: ConversationId) => Promise<void>;
  autoWork: AutoWorkDraftValue;
  /** Whether the selected target may receive the staged workspace resource. */
  workspaceEnabled: boolean;
  /** Stable preset capability/resource resolution must finish before launch. */
  resourceResolutionReady: boolean;
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

/** Creates either a plain Nomi conversation or a frozen AgentPreset Session. */
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
    selection,
    selectedPreset,
    current_model,
    applyAdvancedConfig,
    autoWork,
    workspaceEnabled,
    resourceResolutionReady,
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
  const selectedWorkspace = workspaceEnabled ? dir : '';

  const handleSend = useCallback(async () => {
    const entryPlan = planGuidEntry(input, autoWork);
    let conversationId: ConversationId;
    let conversation;

    if (selection.kind === 'default') {
      if (!current_model) throw new Error('MODEL_REQUIRED');
      conversation = await ipcBridge.conversation.create.invoke({
        type: 'nomi',
        name: entryPlan.conversationName,
        model: current_model,
        extra: {
          default_files: files,
          workspace: selectedWorkspace,
          custom_workspace: Boolean(selectedWorkspace),
        },
      });
      if (!conversation?.id) {
        throw new Error('Nomi conversation was not created');
      }
      conversationId = conversation.id;
    } else {
      if (
        !selectedPreset?.current_stable_revision ||
        selectedPreset.preset_id !== selection.presetId
      ) {
        throw new Error('AGENT_PRESET_REQUIRED');
      }
      const session = await ipcBridge.agentPlatform.sessions.create.invoke({
        preset_id: selectedPreset.preset_id,
        title: entryPlan.conversationName,
      });
      conversationId = parseConversationId(session.agent_session_id);
      conversation = await ipcBridge.conversation.get.invoke({
        conversation_id: conversationId,
      });
      if (!conversation?.id) {
        throw new Error(
          'AgentSession was created without a Conversation projection'
        );
      }
      if (selectedWorkspace) {
        const updated = await ipcBridge.conversation.update.invoke({
          conversation_id: conversationId,
          updates: { extra: { workspace: selectedWorkspace } },
        });
        if (!updated) {
          throw new Error('AgentSession workspace was not bound');
        }
        conversation = await ipcBridge.conversation.get.invoke({
          conversation_id: conversationId,
        });
        if (!conversation?.id) {
          throw new Error(
            'AgentSession workspace update lost its Conversation projection'
          );
        }
      }
    }

    await applyAdvancedConfig?.(conversationId);
    emitter.emit('chat.history.refresh');

    if (entryPlan.sendInitialMessage) {
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

    seedConversationCache(conversation);
    await navigate(`/conversation/${conversationId}`);
  }, [
    applyAdvancedConfig,
    autoWork,
    current_model,
    files,
    input,
    navigate,
    selectedWorkspace,
    selection,
    selectedPreset,
  ]);

  const sendMessageHandler = useCallback(() => {
    if (loading || sendingRef.current) return;
    if (!resourceResolutionReady) return;
    if (selection.kind === 'default' && !current_model) {
      Message.warning(t('conversation.noModelConfigured'));
      return;
    }
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

  const hasLaunchTarget =
    selection.kind === 'default'
      ? Boolean(current_model)
      : Boolean(
          selectedPreset?.current_stable_revision &&
            selectedPreset.preset_id === selection.presetId &&
            resourceResolutionReady
        );
  const isButtonDisabled = loading || !input.trim() || !hasLaunchTarget;

  return {
    handleSend,
    sendMessageHandler,
    isButtonDisabled,
  };
};
