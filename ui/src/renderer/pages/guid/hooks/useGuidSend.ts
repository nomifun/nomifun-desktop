/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 * Based on AionUi (https://github.com/iOfficeAI/AionUi)
 */

import { ipcBridge } from '@/common';
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
import type { ExecutableAgentPreset } from '../types';
import { isAutoWorkEntry, planGuidEntry } from './autoWorkEntry';

export type GuidSendDeps = {
  input: string;
  setInput: React.Dispatch<React.SetStateAction<string>>;
  files: string[];
  setFiles: React.Dispatch<React.SetStateAction<string[]>>;
  setDir: React.Dispatch<React.SetStateAction<string>>;
  setLoading: React.Dispatch<React.SetStateAction<boolean>>;
  loading: boolean;
  selectedPreset: ExecutableAgentPreset | undefined;
  applyAdvancedConfig?: (conversationId: ConversationId) => Promise<void>;
  autoWork: AutoWorkDraftValue;
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

/** Creates a Session from one saved AgentPreset and stages its first message. */
export const useGuidSend = (deps: GuidSendDeps): GuidSendResult => {
  const {
    input,
    setInput,
    files,
    setFiles,
    setDir,
    setLoading,
    loading,
    selectedPreset,
    applyAdvancedConfig,
    autoWork,
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
    if (!selectedPreset) throw new Error('AGENT_PRESET_REQUIRED');

    const entryPlan = planGuidEntry(input, autoWork);
    const session = await ipcBridge.agentPlatform.sessions.create.invoke({
      preset_id: selectedPreset.preset_id,
      title: entryPlan.conversationName,
    });
    const conversationId = parseConversationId(session.agent_session_id);
    const conversation = await ipcBridge.conversation.get.invoke({
      conversation_id: conversationId,
    });
    if (!conversation?.id) {
      throw new Error(
        'AgentSession was created without a Conversation projection'
      );
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
    files,
    input,
    navigate,
    selectedPreset,
  ]);

  const sendMessageHandler = useCallback(() => {
    if (loading || sendingRef.current) return;
    if (!selectedPreset?.current_stable_revision) {
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
        console.error('Failed to create AgentPreset conversation:', error);
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

  const isButtonDisabled =
    loading || !input.trim() || !selectedPreset?.current_stable_revision;

  return {
    handleSend,
    sendMessageHandler,
    isButtonDisabled,
  };
};
