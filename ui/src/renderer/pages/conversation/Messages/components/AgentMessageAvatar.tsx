/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import useSWR from 'swr';
import { usePresetInfo } from '@renderer/hooks/agent/usePresetInfo';
import { getConversationOrNull } from '@/renderer/pages/conversation/utils/conversationCache';
import type { ConversationId } from '@/common/types/ids';

type Props = {
  senderName: string;
  /** Sender Agent's conversation id — enables preset-aware avatar resolution via conversation extras. */
  senderConversationId?: ConversationId;
  /** Precomputed backend logo URL (fallback when no preset avatar is found). */
  backendLogo: string | null;
};

/**
 * Avatar shown next to a participating Agent's message bubble. Prefers the
 * sender's preset icon (emoji or svg) over the generic backend logo.
 */
const AgentMessageAvatar: React.FC<Props> = ({ senderName, senderConversationId, backendLogo }) => {
  const { data: conversation } = useSWR(senderConversationId ? ['agent-conversation', senderConversationId] : null, () =>
    getConversationOrNull(senderConversationId!)
  );
  const { info: presetInfo } = usePresetInfo(conversation ?? undefined);

  if (presetInfo) {
    if (presetInfo.isEmoji) {
      return (
        <span className='w-20px h-20px rounded-full flex items-center justify-center text-14px leading-none bg-fill-2'>
          {presetInfo.logo}
        </span>
      );
    }
    return <img src={presetInfo.logo} alt={presetInfo.name} className='w-20px h-20px rounded-full object-contain' />;
  }

  if (backendLogo) {
    return <img src={backendLogo} alt={senderName} className='w-20px h-20px rounded-full object-contain' />;
  }

  return (
    <div className='w-20px h-20px rounded-full bg-fill-3 flex items-center justify-center text-10px text-t-secondary font-medium'>
      {senderName.charAt(0).toUpperCase()}
    </div>
  );
};

export default AgentMessageAvatar;
