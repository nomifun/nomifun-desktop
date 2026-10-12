/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type { TMessage } from '@/common/chat/chatLib';
import type { ConversationId } from '@/common/types/ids';

export const EXPORT_IO_TIMEOUT_MS = 15000;

export const withTimeout = async <T>(promise: Promise<T>, timeoutMs: number, label: string): Promise<T> => {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      promise,
      new Promise<T>((_, reject) => {
        timer = setTimeout(() => reject(new Error(`${label} timeout`)), timeoutMs);
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
};

const compareMessages = (left: TMessage, right: TMessage): number => {
  const difference = (left.created_at ?? 0) - (right.created_at ?? 0);
  if (difference) return difference;
  const leftId = left.message_id ?? '';
  const rightId = right.message_id ?? '';
  return leftId < rightId ? -1 : leftId > rightId ? 1 : 0;
};

/** Read every canonical history window. The backend caps pages at 500 and
 * advances history by a keyset cursor, irrespective of the requested page. */
export const fetchConversationExportMessages = async (
  conversation_id: ConversationId,
  assertActive: () => void = () => {}
): Promise<TMessage[]> => {
  const messages: TMessage[] = [];
  let cursor = '';
  while (true) {
    assertActive();
    const result = await withTimeout(
      ipcBridge.database.getConversationMessages.invoke({
        conversation_id,
        cursor,
        page_size: 500,
        content_mode: 'full',
      }),
      EXPORT_IO_TIMEOUT_MS,
      `getConversationMessages:${conversation_id}`
    );
    assertActive();
    messages.push(...result.items);
    if (!result.has_more) return messages.toSorted(compareMessages);

    const nextCursor = result.next_cursor;
    if (!nextCursor) {
      throw new Error('Conversation export history is missing its pagination cursor');
    }
    const nextTimestamp = Number(nextCursor.split(':')[0]);
    if (!/^\d+:[^:]+$/.test(nextCursor) || !Number.isSafeInteger(nextTimestamp) || nextCursor === cursor || (cursor && nextTimestamp >= Number(cursor.split(':')[0]))) {
      throw new Error('Conversation export history cursor did not advance');
    }
    cursor = nextCursor;
  }
};
