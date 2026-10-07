/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { AutoWorkDraftValue } from '@/renderer/pages/conversation/components/AutoWorkControl';

export interface GuidEntryPlan {
  /** True when this entry starts an AutoWork session rather than a chat send. */
  autoWorkEntry: boolean;
  /**
   * Whether to store + send the typed input as the conversation's first message.
   * For an AutoWork entry this is `false`: AutoWork drives the conversation from
   * the requirement tag, so sending a first message would start a second turn
   * that races the AutoWork turn and loses with
   * "conversation N is already running".
   */
  sendInitialMessage: boolean;
  /** Conversation name to create with. */
  conversationName: string;
}

/** An AutoWork entry requires the switch ON *and* a chosen requirement tag. */
export function isAutoWorkEntry(autoWork: AutoWorkDraftValue): boolean {
  return autoWork.enabled && !!autoWork.tag;
}

/** Titles share the backend's 200 UTF-8 byte contract; input remains intact. */
const conversationTitle = (input: string): string => {
  const firstLine = input.trim().split(/[\r\n]/, 1)[0];
  const encoder = new TextEncoder();
  let title = '';
  let bytes = 0;
  for (const character of firstLine) {
    const size = encoder.encode(character).length;
    if (bytes + size > 200) break;
    title += character;
    bytes += size;
  }
  return title.trim();
};

/**
 * Decide how the Guid page should enter a conversation, given the typed input
 * and the AutoWork draft. Centralizes the rule that an AutoWork entry must not
 * also send an initial message (the root cause of the duplicate-turn toast).
 */
export function planGuidEntry(input: string, autoWork: AutoWorkDraftValue): GuidEntryPlan {
  const autoWorkEntry = isAutoWorkEntry(autoWork);
  return {
    autoWorkEntry,
    sendInitialMessage: !autoWorkEntry,
    conversationName: conversationTitle(autoWorkEntry ? input.trim() || (autoWork.tag ?? '') : input),
  };
}

/** Whether the "Start AutoWork" primary button is disabled. */
export function autoWorkStartDisabled(loading: boolean, autoWork: AutoWorkDraftValue): boolean {
  return loading || !isAutoWorkEntry(autoWork);
}
