/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */
import type { ConversationId } from '@/common/types/ids';

import { getConversationOrNull } from '@/renderer/pages/conversation/utils/conversationCache';
import { useEffect, useState } from 'react';

/**
 * Resolve the backend turn-start timestamp (epoch ms) for the running turn of a
 * conversation, so the processing/elapsed indicator can keep counting across
 * view unmount/remount (tab switch, session switch) or reconnection instead of
 * restarting from zero.
 *
 * Pass the same `running` boolean that gates the indicator's visibility. While
 * running, this reads the conversation's runtime summary and returns
 * `processing_started_at` when the backend reports an active turn. When not
 * running it returns undefined (cleared), so a subsequent locally-initiated turn
 * correctly anchors to its own start (the consumer falls back to mount time)
 * rather than reusing a stale previous-turn timestamp.
 */
export function useProcessingStartedAt(conversation_id: ConversationId, running: boolean): number | undefined {
  const [startedAt, setStartedAt] = useState<number | undefined>(undefined);

  useEffect(() => {
    if (!running) {
      setStartedAt(undefined);
      return;
    }

    let cancelled = false;
    void getConversationOrNull(conversation_id).then((conversation) => {
      if (cancelled) return;
      const runtime = conversation?.runtime;
      if (runtime?.is_processing && typeof runtime.processing_started_at === 'number') {
        setStartedAt(runtime.processing_started_at);
      } else {
        setStartedAt(undefined);
      }
    });

    return () => {
      cancelled = true;
    };
  }, [conversation_id, running]);

  return startedAt;
}
