/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

export type ScheduledCreateTarget = {
  kind: 'conversation';
  conversationId: number;
};

export function parseScheduledCreateTarget(searchParams: URLSearchParams): ScheduledCreateTarget | null {
  if (searchParams.get('create') !== 'conversation') return null;

  const rawConversationId = searchParams.get('conversation_id') ?? searchParams.get('conversationId');
  if (!rawConversationId) return null;

  const conversationId = Number(rawConversationId);
  if (!Number.isInteger(conversationId) || conversationId <= 0) return null;

  return { kind: 'conversation', conversationId };
}
