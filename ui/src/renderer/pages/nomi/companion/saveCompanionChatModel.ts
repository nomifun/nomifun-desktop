/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type { TChatConversation } from '@/common/config/storage';
import type { ProviderId } from '@/common/types/ids';
import type { CompanionHandle } from '../workspace/types';

export type CompanionChatModel = { provider_id: ProviderId; model: string };
export type CompanionChatModelSaveResult = { status: 'saved' } | { status: 'default-save-failed' };

/**
 * The Session's canonical model changes first. Profile defaults never stand in
 * for the current binding, and a retry after a default-save failure can supply
 * the committed Session model without repeating its transition.
 */
export async function saveCompanionChatModel({
  conversation,
  sessionModel,
  profileModel,
  next,
  patchCompanion,
  onSessionChanged,
}: {
  conversation?: Pick<TChatConversation, 'id'> | null;
  sessionModel?: CompanionChatModel | null;
  profileModel?: CompanionChatModel | null;
  next: CompanionChatModel;
  patchCompanion: CompanionHandle['patchCompanion'];
  onSessionChanged?: (next: CompanionChatModel) => void;
}): Promise<CompanionChatModelSaveResult> {
  if (conversation && (sessionModel?.provider_id !== next.provider_id || sessionModel.model !== next.model)) {
    const switched = await ipcBridge.conversation.switchModel.invoke({
      conversation_id: conversation.id,
      provider_id: next.provider_id,
      model: next.model,
    });
    if (!switched) throw new Error('Session model switch was not committed');
    onSessionChanged?.(next);
  }

  if (profileModel?.provider_id === next.provider_id && profileModel.model === next.model) return { status: 'saved' };
  try {
    const saved = await patchCompanion({ model: next }, { optimistic: false });
    if (!saved) throw new Error('Companion model default was not saved');
    return { status: 'saved' };
  } catch (error) {
    if (!conversation) throw error;
    return { status: 'default-save-failed' };
  }
}
