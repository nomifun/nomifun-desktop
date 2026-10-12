/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type { ConversationId, MessageId } from '@/common/types/ids';
import type { TurnFileSummary } from '@/common/types/platform/turnFiles';
import { addEventListener } from '@/renderer/utils/emitter';
import { useEffect, useMemo, useState } from 'react';

/**
 * Read files by exact Session and closed Turn, independently of message pages.
 * Scope cleanup fences navigation and newly running/resumed Turns. Reads are
 * serialized; repeated settlement notifications coalesce into one follow-up.
 */
export function useTurnFiles(conversationId: ConversationId | undefined, closedTurnIds: MessageId[]) {
  const turnKey = [...new Set(closedTurnIds)].sort().join(',');
  const [state, setState] = useState<{
    conversationId: ConversationId;
    summaries: Map<MessageId, TurnFileSummary>;
  }>();

  useEffect(() => setState(undefined), [conversationId]);
  useEffect(() => {
    const scope = new Set(turnKey.split(','));
    setState((current) => {
      if (!current || [...current.summaries.keys()].every(id => scope.has(id))) return current;
      return { ...current, summaries: new Map([...current.summaries].filter(([id]) => scope.has(id))) };
    });
  }, [turnKey]);

  useEffect(() => {
    if (!conversationId || !turnKey) return;
    const ids = turnKey.split(',') as MessageId[];
    const scope = new Set(ids);
    let disposed = false;
    let reading = false;
    let requested = false;
    let cancelRead: (() => void) | undefined;
    let retry: ReturnType<typeof setTimeout> | undefined;

    const refresh = () => {
      if (disposed) return;
      requested = true;
      if (reading) return;
      reading = true;
      clearTimeout(retry);
      void (async () => {
        let failed = false;
        while (requested && !disposed) {
          requested = false;
          let timeout: ReturnType<typeof setTimeout> | undefined;
          let abandoned = false;
          try {
            const read = async () => {
              const result: TurnFileSummary[] = [];
              // The endpoint accepts at most 100 roots. Load additional history
              // in batches instead of turning a large transcript into a 400.
              for (let offset = 0; offset < ids.length; offset += 100) {
                if (disposed || abandoned) throw new Error('Turn file read superseded');
                result.push(...await ipcBridge.conversation.turnFiles.invoke({
                  conversation_id: conversationId, turn_ids: ids.slice(offset, offset + 100),
                }));
              }
              return result;
            };
            const summaries = await Promise.race([
              read(),
              new Promise<never>((_, reject) => {
                cancelRead = () => { abandoned = true; reject(new Error('Turn file read superseded')); };
                timeout = setTimeout(() => { abandoned = true; reject(new Error('Turn file read timed out')); }, 5_000);
              }),
            ]);
            if (disposed) break;
            if (summaries.some((summary) => !scope.has(summary.turn_id))) {
              throw new Error('Turn file response belongs to another Turn');
            }
            setState({ conversationId, summaries: new Map(summaries.map((summary) => [summary.turn_id, summary])) });
            failed = false;
          } catch (error) {
            failed = true;
            if (!disposed) console.warn('[turn-files] Failed to read file list:', error);
          } finally {
            clearTimeout(timeout);
            cancelRead = undefined;
          }
        }
        reading = false;
        if (failed && !disposed) retry = setTimeout(refresh, 5_000);
      })();
    };
    const offCompleted = ipcBridge.conversation.turnCompleted.on((event) => {
      if (event.conversation_id === conversationId) refresh();
    });
    const offReconnect = ipcBridge.conversation.reconnected.on(refresh);
    const offSettled = addEventListener('conversation.turn.settled', (id) => {
      if (id === conversationId) refresh();
    });
    window.addEventListener('focus', refresh);
    refresh();
    return () => {
      disposed = true;
      clearTimeout(retry);
      cancelRead?.();
      offCompleted(); offReconnect(); offSettled();
      window.removeEventListener('focus', refresh);
    };
  }, [conversationId, turnKey]);

  return useMemo(() => {
    if (!state || state.conversationId !== conversationId) return new Map<MessageId, TurnFileSummary>();
    const scope = new Set(turnKey.split(','));
    return new Map([...state.summaries].filter(([id]) => scope.has(id)));
  }, [conversationId, turnKey, state]);
}
