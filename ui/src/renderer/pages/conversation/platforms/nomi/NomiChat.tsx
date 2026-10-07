/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */
import type { ConversationId, CronJobId } from '@/common/types/ids';

import type { ConversationContextValue } from '@/renderer/hooks/context/ConversationContext';
import { ConversationProvider } from '@/renderer/hooks/context/ConversationContext';
import FlexFullContainer from '@renderer/components/layout/FlexFullContainer';
import MessageList from '@renderer/pages/conversation/Messages/MessageList';
import ConversationPluginArtifacts from '@/renderer/pages/plugins/ConversationPluginArtifacts';
import { PLUGIN_FEATURE_VISIBLE } from '@/renderer/utils/plugins/pluginFeatureAvailability';
import {
  MessageListLoadingProvider,
  MessageListProvider,
  useMessageLstCache,
} from '@renderer/pages/conversation/Messages/hooks';
import HOC from '@renderer/utils/ui/HOC';
import React, { useEffect, useMemo } from 'react';
import LocalImageView from '@renderer/components/media/LocalImageView';
import NomiSendBox from './NomiSendBox';
import { useNomiMessage } from './useNomiMessage';
import type { NomiModelSelection } from './useNomiModelSelection';
import { ConversationCreationTasksProvider } from '@/renderer/creation/ConversationCreationTasks';
import type { SessionReasoningEffort } from '@/common/types/reasoningEffort';
import { currentModelProviderTarget } from './currentModelProviderTarget';

const NomiChat: React.FC<{
  conversation_id: ConversationId;
  workspace: string;
  modelSelection: NomiModelSelection;
  agentSelectorNode?: React.ReactNode;
  cron_job_id?: CronJobId;
  hideSendBox?: boolean;
  readOnly?: boolean;
  emptySlot?: React.ReactNode;
  agent_name?: string;
  currentAgent?: ConversationContextValue['currentAgent'];
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
  /** Product-owned compact composer; configuration is exposed outside chat. */
  compactProductComposer?: boolean;
}> = ({
  conversation_id,
  workspace,
  modelSelection,
  agentSelectorNode,
  cron_job_id,
  hideSendBox,
  readOnly,
  emptySlot,
  agent_name,
  currentAgent,
  isProcessing,
  modelSelectionHint,
  modelSelectionDisabled,
  reasoningEffort,
  reasoningEffortUpdating,
  onReasoningEffortChange,
  extraRightTools,
  creationTasksEnabled = false,
  creationEnabled = true,
  compactProductComposer = false,
}) => {
  // Windowed history: load only the newest page on mount + lazily prepend older
  // pages on scroll-up. The nomi surface backs both work conversations and the
  // companion's single session (which also absorbs every IM-channel turn and can
  // grow without bound), so a one-shot 10k fetch would crush the API/DOM.
  const historyPaging = useMessageLstCache(conversation_id);
  const turnActivity = useNomiMessage(conversation_id);
  const updateLocalImage = LocalImageView.useUpdateLocalImage();
  useEffect(() => {
    updateLocalImage({ root: workspace });
  }, [workspace]);
  const resolvedIsProcessing = turnActivity.hasHydratedRunningState
    ? turnActivity.running
    : isProcessing === true || turnActivity.running;
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
      activeTurnId: turnActivity.activeTurnId,
      activeRequestMessageId: turnActivity.activeRequestMessageId,
      stopNotice: turnActivity.stopNotice,
      executionPause: turnActivity.pauseNotice,
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
    turnActivity.activeTurnId,
    turnActivity.activeRequestMessageId,
    turnActivity.stopNotice,
    turnActivity.pauseNotice,
    currentAgent,
    currentModel,
  ]);

  return (
    <ConversationProvider value={conversationValue}>
      <ConversationCreationTasksProvider conversationId={conversation_id} enabled={creationTasksEnabled}>
        <div data-conversation-layout className='flex-1 flex flex-col px-20px min-h-0'>
          <FlexFullContainer>
            <MessageList
              className='flex-1'
              emptySlot={emptySlot}
              onLoadOlder={historyPaging.loadOlder}
              hasMoreOlder={historyPaging.hasMore}
              loadingOlder={historyPaging.loadingOlder}
            />
          </FlexFullContainer>
          {!readOnly && PLUGIN_FEATURE_VISIBLE && <ConversationPluginArtifacts conversationId={conversation_id} />}
          {!readOnly && !hideSendBox && (
            <NomiSendBox
              conversation_id={conversation_id}
              modelSelection={modelSelection}
              agentSelectorNode={agentSelectorNode}
              agent_name={agent_name}
              modelSelectionHint={modelSelectionHint}
              modelSelectionDisabled={modelSelectionDisabled}
              reasoningEffort={reasoningEffort}
              reasoningEffortUpdating={reasoningEffortUpdating}
              onReasoningEffortChange={onReasoningEffortChange}
              extraRightTools={extraRightTools}
              creationEnabled={creationEnabled}
              compactProductComposer={compactProductComposer}
              turnActivity={turnActivity}
            />
          )}
        </div>
      </ConversationCreationTasksProvider>
    </ConversationProvider>
  );
};

export default HOC.Wrapper(MessageListProvider, MessageListLoadingProvider, LocalImageView.Provider)(NomiChat);
