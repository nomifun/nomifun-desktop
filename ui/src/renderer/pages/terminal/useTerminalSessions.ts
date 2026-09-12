/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { ipcBridge } from '@/common';
import type { ITerminalSession } from '@/common/adapter/ipcBridge';
import type { TerminalId } from '@/common/types/ids';
import { emitter } from '@/renderer/utils/emitter';

type SessionUpdate = (sessions: ITerminalSession[]) => ITerminalSession[];

/**
 * Live list of standalone, user-owned terminal sessions for the global
 * sidebar. Conversation-owned terminals are intentionally excluded: their
 * lifecycle belongs to the conversation that created them and they are shown
 * in that conversation's right-hand terminal panel instead.
 */
export function useTerminalSessions() {
  const [sessions, setSessions] = useState<ITerminalSession[]>([]);
  const [loading, setLoading] = useState(false);
  const active = useRef(false);
  const pending = useRef<SessionUpdate[] | null>(null);

  const applyUpdate = useCallback((update: SessionUpdate) => {
    if (!active.current) return;
    pending.current?.push(update);
    setSessions(update);
  }, []);

  const refresh = useCallback(async () => {
    if (!active.current) return;
    // The array owns this request and replays live changes over its snapshot.
    const updates: SessionUpdate[] = [];
    pending.current = updates;
    setLoading(true);
    try {
      const list = await ipcBridge.terminal.list.invoke();
      if (pending.current !== updates) return;
      setSessions(updates.reduce((rows, update) => update(rows), Array.isArray(list) ? list : []));
    } catch {
      // Keep live state on failure; otherwise retain the existing empty fallback.
      if (pending.current === updates && updates.length === 0) setSessions([]);
    } finally {
      if (pending.current === updates) {
        pending.current = null;
        setLoading(false);
      }
    }
  }, []);

  useEffect(() => {
    active.current = true;
    const upsert = (s: ITerminalSession) => {
      applyUpdate((prev) => {
        if (s.owner_conversation_id) {
          return prev.filter((p) => p.terminal_id !== s.terminal_id);
        }
        return prev.some((p) => p.terminal_id === s.terminal_id)
          ? prev.map((p) => (p.terminal_id === s.terminal_id ? s : p))
          : [s, ...prev];
      });
    };
    const offCreated = ipcBridge.terminal.onCreated.on(upsert);
    const offUpdated = ipcBridge.terminal.onUpdated.on(upsert);
    const offRemoved = ipcBridge.terminal.onRemoved.on((evt) => {
      applyUpdate((prev) => prev.filter((p) => p.terminal_id !== evt.terminal_id));
    });
    const offExit = ipcBridge.terminal.onExit.on((evt) => {
      applyUpdate((prev) =>
        prev.map((p) =>
          p.terminal_id === evt.terminal_id
            ? { ...p, last_status: 'exited', exit_code: evt.exit_code }
            : p,
        ),
      );
    });
    const offRefresh = (): void => {
      void refresh();
    };
    emitter.on('terminal.list.refresh', offRefresh);
    const offReconnected = ipcBridge.terminal.onReconnected.on(() => {
      void refresh();
    });
    void refresh();

    return () => {
      active.current = false;
      pending.current = null;
      offCreated();
      offUpdated();
      offRemoved();
      offExit();
      offReconnected();
      emitter.off('terminal.list.refresh', offRefresh);
    };
  }, [applyUpdate, refresh]);

  const removeSession = useCallback(async (id: TerminalId) => {
    await ipcBridge.terminal.remove.invoke({ terminal_id: id });
    applyUpdate((prev) => prev.filter((p) => p.terminal_id !== id));
  }, [applyUpdate]);

  return { sessions, loading, refresh, removeSession };
}
