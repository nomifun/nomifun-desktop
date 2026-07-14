/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

export function parseScheduledConversationId(searchParams: URLSearchParams): number | null {
  if (searchParams.get('create') !== 'conversation') return null;

  const rawConversationId = searchParams.get('conversation_id') ?? searchParams.get('conversationId');
  if (!rawConversationId) return null;

  const conversationId = Number(rawConversationId);
  return Number.isInteger(conversationId) && conversationId > 0 ? conversationId : null;
}
