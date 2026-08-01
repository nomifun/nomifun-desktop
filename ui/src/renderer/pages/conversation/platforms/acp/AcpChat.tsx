/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */
import type { ConversationId, CronJobId } from '@/common/types/ids';

import type { IConversationMcpStatus } from '@/common/config/storage';
import { ConversationProvider } from '@/renderer/hooks/context/ConversationContext';
import FlexFullContainer from '@renderer/components/layout/FlexFullContainer';
import MessageList from '@renderer/pages/conversation/Messages/MessageList';
import { ConversationArtifactProvider } from '@renderer/pages/conversation/Messages/artifacts';
import {
  MessageListLoadingProvider,
  MessageListProvider,
  useAddOrUpdateMessage,
  useMessageLstCache,
} from '@renderer/pages/conversation/Messages/hooks';
import { usePendingConfirmationsRecovery } from '@renderer/pages/conversation/Messages/usePendingConfirmationsRecovery';
import { useAutoTitle } from '@/renderer/hooks/chat/useAutoTitle';
import HOC from '@renderer/utils/ui/HOC';
import LocalImageView from '@renderer/components/media/LocalImageView';
import React, { Suspense, useEffect } from 'react';
import AcpSendBox from './AcpSendBox';
import { useAcpInitialMessage } from './useAcpInitialMessage';
import { useAcpMessage } from './useAcpMessage';

// E2E 压测注入器仅在 dev 构建挂载，动态 import 使其不进入生产 bundle
// E2E stream injector is mounted in dev builds only; the dynamic import keeps
// it out of production bundles (the dead branch is eliminated at build time)
const AcpE2EStreamInjector =
  process.env.NODE_ENV === 'development' ? React.lazy(() => import('./AcpE2EStreamInjector')) : null;

const AcpChat: React.FC<{
  conversation_id: ConversationId;
  workspace?: string;
  backend: string;
  initialModelId?: string;
  session_mode?: string;
  agent_name?: string;
  cron_job_id?: CronJobId;
  hideSendBox?: boolean;
  readOnly?: boolean;
  emptySlot?: React.ReactNode;
  loadedSkills?: string[];
  loadedMcpStatuses?: IConversationMcpStatus[];
}> = ({
  conversation_id,
  workspace,
  backend,
  initialModelId,
  session_mode,
  agent_name,
  cron_job_id,
  hideSendBox,
  readOnly,
  emptySlot,
  loadedSkills,
  loadedMcpStatuses,
}) => {
  useMessageLstCache(conversation_id);
  usePendingConfirmationsRecovery(conversation_id, { enabled: !readOnly });
  const { checkAndUpdateTitle } = useAutoTitle();
  const addOrUpdateMessage = useAddOrUpdateMessage();
  const messageState = useAcpMessage(conversation_id, { skipWarmup: readOnly === true });
  const updateLocalImage = LocalImageView.useUpdateLocalImage();
  useEffect(() => {
    updateLocalImage({ root: workspace ?? '' });
  }, [updateLocalImage, workspace]);
  useAcpInitialMessage({
    conversation_id,
    backend,
    workspacePath: workspace,
    enabled: !readOnly,
    setAiProcessing: messageState.setAiProcessing,
    markTurnAccepted: messageState.markTurnAccepted,
    reconcilePublicDeliveryReplay: messageState.reconcilePublicDeliveryReplay,
    checkAndUpdateTitle,
    addOrUpdateMessage,
  });

  return (
    <ConversationProvider
      value={{
        conversation_id: conversation_id,
        workspace,
        type: 'acp',
        cron_job_id,
        hideSendBox,
        readOnly,
        isProcessing: messageState.running,
        activeTurnId: messageState.activeTurnId,
        activeRequestMessageId: messageState.activeRequestMessageId,
        loadedSkills,
        loadedMcpStatuses,
      }}
    >
      <ConversationArtifactProvider conversation_id={conversation_id}>
        <div className='flex-1 flex flex-col px-20px min-h-0'>
          <FlexFullContainer>
            <MessageList className='flex-1' emptySlot={emptySlot} />
          </FlexFullContainer>
          {AcpE2EStreamInjector && (
            <Suspense fallback={null}>
              <AcpE2EStreamInjector conversationId={conversation_id} />
            </Suspense>
          )}
          {!readOnly && !hideSendBox && (
            <AcpSendBox
              conversation_id={conversation_id}
              backend={backend}
              initialModelId={initialModelId}
              session_mode={session_mode}
              agent_name={agent_name}
              workspacePath={workspace}
              messageState={messageState}
            ></AcpSendBox>
          )}
        </div>
      </ConversationArtifactProvider>
    </ConversationProvider>
  );
};

export default HOC.Wrapper(MessageListProvider, MessageListLoadingProvider, LocalImageView.Provider)(AcpChat);
