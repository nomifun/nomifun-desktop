/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */
import type { ConversationId, CronJobId } from '@/common/types/ids';

import { ConversationProvider } from '@/renderer/hooks/context/ConversationContext';
import FlexFullContainer from '@renderer/components/layout/FlexFullContainer';
import MessageList from '@renderer/pages/conversation/Messages/MessageList';
import {
  MessageListLoadingProvider,
  MessageListProvider,
  useMessageLstCache,
} from '@renderer/pages/conversation/Messages/hooks';
import HOC from '@renderer/utils/ui/HOC';
import React, { useEffect } from 'react';
import LocalImageView from '@renderer/components/media/LocalImageView';
import { useConversationResponseMessages } from '@renderer/pages/conversation/Messages/useConversationResponseMessages';

export interface BasicRuntimeChatProps {
  conversation_id: ConversationId;
  workspace: string;
  cron_job_id?: CronJobId;
  hideSendBox?: boolean;
  readOnly?: boolean;
  emptySlot?: React.ReactNode;
  loadedSkills?: string[];
}

/**
 * Shared chat surface for the "basic runtime" platforms
 * (openclaw-gateway).
 *
 * These platforms render an identical message list + send box shell; the only
 * platform-specific parts are the ConversationProvider `type` and the send box
 * component, both supplied here. The stateful ACP / Nomi chat surfaces have
 * their own wiring (warmup, pending confirmations, initial message hooks) and
 * are intentionally not built on this factory.
 */
export function createBasicRuntimeChat(
  type: 'openclaw-gateway',
  PlatformSendBox: React.ComponentType<{ conversation_id: ConversationId }>
) {
  const BasicRuntimeChat: React.FC<BasicRuntimeChatProps> = ({
    conversation_id,
    workspace,
    cron_job_id,
    hideSendBox,
    readOnly,
    emptySlot,
    loadedSkills,
  }) => {
    useMessageLstCache(conversation_id);
    useConversationResponseMessages(conversation_id);
    const updateLocalImage = LocalImageView.useUpdateLocalImage();
    useEffect(() => {
      updateLocalImage({ root: workspace });
    }, [updateLocalImage, workspace]);
    return (
      <ConversationProvider
        value={{ conversation_id: conversation_id, workspace, type, cron_job_id, hideSendBox, readOnly, loadedSkills }}
      >
        <div className='flex-1 flex flex-col px-20px min-h-0'>
          <FlexFullContainer>
            <MessageList className='flex-1' emptySlot={emptySlot}></MessageList>
          </FlexFullContainer>
          {!readOnly && !hideSendBox && <PlatformSendBox conversation_id={conversation_id} />}
        </div>
      </ConversationProvider>
    );
  };
  BasicRuntimeChat.displayName = `BasicRuntimeChat(${type})`;
  return HOC.Wrapper(MessageListProvider, MessageListLoadingProvider, LocalImageView.Provider)(BasicRuntimeChat);
}
