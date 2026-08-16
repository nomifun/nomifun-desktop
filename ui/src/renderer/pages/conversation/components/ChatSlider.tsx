/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { TChatConversation } from '@/common/config/storage';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import React from 'react';
import ChatWorkspace from '../Workspace';
import type { WorkspaceExtraTab } from '../Workspace/types';

const ChatSlider: React.FC<{
  conversation?: TChatConversation;
  extraTabs?: WorkspaceExtraTab[];
}> = ({ conversation, extraTabs }) => {
  const [messageApi, messageContext] = useArcoMessage({ maxCount: 1 });

  // All conversation types ('acp' | 'nomi')
  // render the same workspace rail; eventPrefix is always the conversation type.
  if (!conversation?.extra?.workspace) {
    return <div></div>;
  }

  return (
    <>
      {messageContext}
      <ChatWorkspace
        conversation_id={conversation.id}
        workspace={conversation.extra.workspace}
        isTemporaryWorkspace={
          (conversation.extra as { is_temporary_workspace?: boolean } | undefined)?.is_temporary_workspace
        }
        eventPrefix={conversation.type}
        messageApi={messageApi}
        extraTabs={extraTabs}
      ></ChatWorkspace>
    </>
  );
};

export default ChatSlider;
