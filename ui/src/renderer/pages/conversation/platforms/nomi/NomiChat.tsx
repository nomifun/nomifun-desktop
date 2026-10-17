/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */
import type { TChatConversation } from '@/common/config/storage';

import type { ConversationContextValue } from '@/renderer/hooks/context/ConversationContext';
import { ConversationProvider } from '@/renderer/hooks/context/ConversationContext';
import { useAgentInfo } from '@/renderer/hooks/agent/useAgentInfo';
import FlexFullContainer from '@renderer/components/layout/FlexFullContainer';
import MessageList from '@renderer/pages/conversation/Messages/MessageList';
import {
  MessageListLoadingProvider,
  MessageListProvider,
  useMessageLstCache,
} from '@renderer/pages/conversation/Messages/hooks';
import HOC from '@renderer/utils/ui/HOC';
import React, { useCallback, useEffect, useMemo } from 'react';
import LocalImageView from '@renderer/components/media/LocalImageView';
import NomiSendBox from './NomiSendBox';
import { useNomiMessage } from './useNomiMessage';
import type { NomiModelSelection } from './useNomiModelSelection';
import { ConversationCreationTasksProvider } from '@/renderer/creation/ConversationCreationTasks';
import type { SessionReasoningEffort } from '@/common/types/reasoningEffort';
import { currentModelProviderTarget } from './currentModelProviderTarget';
import { useInitialMessage } from '../../components/ConversationShell/InitialMessageContext';
import type { IMessageText } from '@/common/chat/chatLib';
import { buildDisplayMessage } from '@/renderer/utils/file/messageFiles';

const NomiChat: React.FC<{
  /** All surfaces derive Session identity and Agent presentation here. */
  conversation: Extract<TChatConversation, { type: 'nomi' }>;
  modelSelection: NomiModelSelection;
  agentSelectorNode?: React.ReactNode;
  hideSendBox?: boolean;
  readOnly?: boolean;
  emptySlot?: React.ReactNode;
  agent_name?: string;
  isProcessing?: boolean;
  modelSelectionHint?: string;
  modelSelectionDisabled?: boolean;
  reasoningEffort?: SessionReasoningEffort;
  reasoningEffortUpdating?: boolean;
  onReasoningEffortChange?: (value: SessionReasoningEffort | undefined) => Promise<void> | void;
  /** Extra right-side tools used by projected task transcripts. */
  extraRightTools?: React.ReactNode;
  /** Only Sessions frozen with creation.media own generation task history. */
  creationTasksEnabled?: boolean;
  /** Product-owned chat surfaces can explicitly suppress media creation scenes. */
  creationEnabled?: boolean;
  collaborationEnabled?: boolean;
  /** Product-owned compact composer; configuration is exposed outside chat. */
  compactProductComposer?: boolean;
  hideSessionCapabilities?: boolean;
  submissionDisabled?: boolean;
  onBeforeSubmit?: () => Promise<void>;
  /** Keep a followed Turn's final reply visible on product chat surfaces. */
  followTurnCompletion?: boolean;
}> = ({
  conversation,
  modelSelection,
  agentSelectorNode,
  hideSendBox,
  readOnly,
  emptySlot,
  agent_name,
  isProcessing,
  modelSelectionHint,
  modelSelectionDisabled,
  reasoningEffort,
  reasoningEffortUpdating,
  onReasoningEffortChange,
  extraRightTools,
  creationTasksEnabled = false,
  creationEnabled = true,
  collaborationEnabled = false,
  compactProductComposer = false,
  hideSessionCapabilities = false,
  submissionDisabled = false,
  onBeforeSubmit,
  followTurnCompletion = false,
}) => {
  const { id: conversation_id, cron_job_id } = conversation;
  const workspace = conversation.extra.workspace ?? '';
  const { info: agentInfo } = useAgentInfo(conversation);
  const currentAgent = useMemo(() => agentInfo
    ? { presetId: agentInfo.preset_id, label: agentInfo.name }
    : undefined, [agentInfo]);
  // Windowed history: load only the newest page on mount + lazily prepend older
  // pages on scroll-up. The nomi surface backs both work conversations and the
  // companion's single session (which also absorbs every IM-channel turn and can
  // grow without bound), so a one-shot 10k fetch would crush the API/DOM.
  const historyPaging = useMessageLstCache(conversation_id);
  const turnActivity = useNomiMessage(conversation_id);
  const { pending, end: endInitialMessage } = useInitialMessage();
  const handlePendingMessageReady = useCallback(() => {
    if (pending?.conversation_id === conversation_id) endInitialMessage(conversation_id, pending.idempotency_key);
  }, [pending, conversation_id, endInitialMessage]);
  const pendingMessage = useMemo<IMessageText | undefined>(() => pending?.conversation_id === conversation_id
    ? {
        id: `initial-${pending.idempotency_key}`,
        conversation_id,
        type: 'text',
        position: 'right',
        status: 'pending',
        created_at: pending.submittedAt,
        content: { content: buildDisplayMessage(pending.input, pending.files, workspace) },
      }
    : undefined, [pending, conversation_id, workspace]);
  const updateLocalImage = LocalImageView.useUpdateLocalImage();
  useEffect(() => {
    updateLocalImage({ root: workspace });
  }, [workspace]);
  const resolvedIsProcessing = !turnActivity.waitingNotice && (turnActivity.hasHydratedRunningState
    ? turnActivity.running
    : isProcessing === true || turnActivity.running);
  const currentModel = useMemo(
    () => currentModelProviderTarget(modelSelection.current_model, modelSelection.providers),
    [modelSelection.current_model?.id, modelSelection.current_model?.use_model, modelSelection.providers]
  );
  const conversationValue = useMemo<ConversationContextValue>(() => {
    return {
      conversation_id: conversation_id,
      workspace,
      type: 'nomi',
      cron_job_id,
      hideSendBox,
      readOnly,
      isProcessing: resolvedIsProcessing,
      isTurnStateHydrated: turnActivity.hasHydratedRunningState,
      activeTurnId: turnActivity.activeTurnId,
      activeTurnStartedAt: turnActivity.activeTurnStartedAt,
      activeRequestMessageId: turnActivity.activeRequestMessageId,
      stopNotice: turnActivity.stopNotice,
      executionPause: turnActivity.pauseNotice,
      executionWait: turnActivity.waitingNotice,
      currentAgent,
      currentModel,
    };
  }, [
    conversation_id,
    workspace,
    cron_job_id,
    hideSendBox,
    readOnly,
    resolvedIsProcessing,
    turnActivity.hasHydratedRunningState,
    turnActivity.activeTurnId,
    turnActivity.activeTurnStartedAt,
    turnActivity.activeRequestMessageId,
    turnActivity.stopNotice,
    turnActivity.pauseNotice,
    turnActivity.waitingNotice,
    currentAgent,
    currentModel,
  ]);

  return (
    <ConversationProvider value={conversationValue}>
      <ConversationCreationTasksProvider conversationId={conversation_id} enabled={creationTasksEnabled}>
        <div data-conversation-layout className='flex-1 flex flex-col min-h-0'>
          <FlexFullContainer>
            <MessageList
              className='flex-1'
              emptySlot={emptySlot}
              pendingMessage={pendingMessage}
              onPendingMessageReady={handlePendingMessageReady}
              onLoadOlder={historyPaging.loadOlder}
              hasMoreOlder={historyPaging.hasMore}
              loadingOlder={historyPaging.loadingOlder}
              followTurnCompletion={followTurnCompletion}
            />
          </FlexFullContainer>
          {!readOnly && !hideSendBox && (
            <NomiSendBox
              conversation_id={conversation_id}
              modelSelection={modelSelection}
              agentSelectorNode={agentSelectorNode}
              agent_name={agent_name ?? agentInfo?.name}
              modelSelectionHint={modelSelectionHint}
              modelSelectionDisabled={modelSelectionDisabled}
              reasoningEffort={reasoningEffort}
              reasoningEffortUpdating={reasoningEffortUpdating}
              onReasoningEffortChange={onReasoningEffortChange}
              extraRightTools={extraRightTools}
              creationEnabled={creationEnabled}
              collaborationEnabled={collaborationEnabled}
              compactProductComposer={compactProductComposer}
              hideSessionCapabilities={hideSessionCapabilities}
              submissionDisabled={submissionDisabled}
              onBeforeSubmit={onBeforeSubmit}
              turnActivity={turnActivity}
            />
          )}
        </div>
      </ConversationCreationTasksProvider>
    </ConversationProvider>
  );
};

export default HOC.Wrapper(MessageListProvider, MessageListLoadingProvider, LocalImageView.Provider)(NomiChat);
