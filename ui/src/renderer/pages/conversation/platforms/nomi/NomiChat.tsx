/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */
import type { ConversationId, CronJobId } from '@/common/types/ids';

import type { IConversationMcpStatus } from '@/common/config/storage';
import type { ConversationContextValue } from '@/renderer/hooks/context/ConversationContext';
import { ConversationProvider } from '@/renderer/hooks/context/ConversationContext';
import FlexFullContainer from '@renderer/components/layout/FlexFullContainer';
import MessageList from '@renderer/pages/conversation/Messages/MessageList';
import { ConversationArtifactProvider } from '@renderer/pages/conversation/Messages/artifacts';
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

const NomiChat: React.FC<{
  conversation_id: ConversationId;
  workspace: string;
  modelSelection: NomiModelSelection;
  cron_job_id?: CronJobId;
  hideSendBox?: boolean;
  readOnly?: boolean;
  emptySlot?: React.ReactNode;
  loadedSkills?: string[];
  loadedMcpStatuses?: IConversationMcpStatus[];
  agent_name?: string;
  isProcessing?: boolean;
  /** Hide model and other editable controls on locked surfaces. */
  hideAdvancedControls?: boolean;
  /** Conversation collaborator-model control rendered after the main model. */
  collaboratorSelectorNode?: React.ReactNode;
  /** Extra right-side tools used by projected task transcripts. */
  extraRightTools?: React.ReactNode;
}> = ({
  conversation_id,
  workspace,
  modelSelection,
  cron_job_id,
  hideSendBox,
  readOnly,
  emptySlot,
  loadedSkills,
  loadedMcpStatuses,
  agent_name,
  isProcessing,
  hideAdvancedControls,
  collaboratorSelectorNode,
  extraRightTools,
}) => {
  // Windowed history: load only the newest page on mount + lazily prepend older
  // pages on scroll-up. The nomi surface backs both work conversations and the
  // companion's single session (which also absorbs every IM-channel turn and can
  // grow without bound), so a one-shot 10k fetch would crush the API/DOM.
  const historyPaging = useMessageLstCache(conversation_id, { windowed: true });
  const turnActivity = useNomiMessage(conversation_id, { readOnly });
  const updateLocalImage = LocalImageView.useUpdateLocalImage();
  useEffect(() => {
    updateLocalImage({ root: workspace });
  }, [workspace]);
  const resolvedIsProcessing = turnActivity.hasHydratedRunningState
    ? turnActivity.running
    : isProcessing === true || turnActivity.running;
  const conversationValue = useMemo<ConversationContextValue>(() => {
    return {
      conversation_id: conversation_id,
      workspace,
      type: 'nomi',
      cron_job_id,
      hideSendBox,
      readOnly,
      isProcessing: resolvedIsProcessing,
      stopNotice: turnActivity.stopNotice,
      loadedSkills,
      loadedMcpStatuses,
    };
  }, [
    conversation_id,
    workspace,
    cron_job_id,
    hideSendBox,
    readOnly,
    resolvedIsProcessing,
    turnActivity.stopNotice,
    loadedSkills,
    loadedMcpStatuses,
  ]);

  return (
    <ConversationProvider value={conversationValue}>
      <ConversationArtifactProvider conversation_id={conversation_id}>
        <div className='flex-1 flex flex-col px-20px min-h-0'>
          <FlexFullContainer>
            <MessageList
              className='flex-1'
              emptySlot={emptySlot}
              onLoadOlder={historyPaging.loadOlder}
              hasMoreOlder={historyPaging.hasMore}
              loadingOlder={historyPaging.loadingOlder}
            />
          </FlexFullContainer>
          {!readOnly && !hideSendBox && (
            <NomiSendBox
              conversation_id={conversation_id}
              modelSelection={modelSelection}
              agent_name={agent_name}
              hideAdvancedControls={hideAdvancedControls}
              collaboratorSelectorNode={collaboratorSelectorNode}
              extraRightTools={extraRightTools}
              turnActivity={turnActivity}
            />
          )}
        </div>
      </ConversationArtifactProvider>
    </ConversationProvider>
  );
};

export default HOC.Wrapper(MessageListProvider, MessageListLoadingProvider, LocalImageView.Provider)(NomiChat);
