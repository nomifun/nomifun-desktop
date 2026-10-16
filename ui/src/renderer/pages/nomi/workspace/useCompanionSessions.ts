/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { useSearchParams } from 'react-router-dom';
import useSWR from 'swr';
import { ipcBridge } from '@/common';
import type { CompanionId, ConversationId } from '@/common/types/ids';
import { parseConversationId } from '@/common/types/ids';
import { uuidv7 } from '@/common/utils';
import { browserStorageKey } from '@/common/utils/browserStorageKey';
import { refreshConversationCache } from '@/renderer/pages/conversation/utils/conversationCache';

/** Navigation is local to this view. Only continueInWindow changes desktop input routing. */
export function useCompanionSessions(companionId: CompanionId, modelConfigured: boolean) {
  const [params, setParams] = useSearchParams();
  const requested = params.get('session');
  const identity = `${companionId}/${requested ?? ''}`;
  const currentIdentity = useRef(identity);
  currentIdentity.current = identity;
  const [creating, setCreating] = useState(false);
  const [routing, setRouting] = useState(false);
  const [actionError, setActionError] = useState(false);
  const [bootstrapError, setBootstrapError] = useState(false);
  const [managing, setManaging] = useState(false);
  const managementPending = useRef(false);
  const creatingRef = useRef(false);
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  useEffect(() => { setActionError(false); setBootstrapError(false); }, [identity]);
  const list = useSWR(['companion-sessions', companionId], () => ipcBridge.companion.listCompanionSessions.invoke({ companion_id: companionId }),
    { shouldRetryOnError: false });
  const target = useSWR(['companion-desktop-target', companionId], () => ipcBridge.companion.getCompanionSession.invoke({ companion_id: companionId }),
    { shouldRetryOnError: false });
  const refresh = useCallback(() => {
    void list.mutate().catch(() => {});
    void target.mutate().catch(() => {});
  }, [list.mutate, target.mutate]);
  useEffect(() => {
    const disposers = [ipcBridge.conversation.listChanged.on(refresh), ipcBridge.conversation.reconnected.on(refresh),
      ipcBridge.companion.onConfigUpdated.on(event => { if (event.scope === companionId) refresh(); })];
    return () => disposers.forEach(dispose => dispose());
  }, [companionId, refresh]);
  const select = useCallback((id: ConversationId, replace = false) => {
    setParams(previous => {
      const next = new URLSearchParams(previous);
      next.set('companion', companionId);
      next.set('mode', 'cohabit');
      next.set('session', id);
      return next;
    }, { replace });
  }, [companionId, setParams]);
  useEffect(() => {
    if (requested || !target.data || target.error) return;
    if (target.data.conversation_id) { select(target.data.conversation_id, true); return; }
    if (!modelConfigured) return;
    let cancelled = false;
    void ipcBridge.companion.ensureCompanionSession.invoke({ companion_id: companionId }).then(session => {
      if (cancelled) return;
      select(session.conversation_id, true);
      refresh();
    }).catch(() => { if (!cancelled) setBootstrapError(true); });
    return () => { cancelled = true; };
  }, [companionId, modelConfigured, requested, target.data, target.error, select, refresh]);
  let conversationId: ConversationId | null = null;
  let invalidSelection = false;
  if (requested && requested !== 'new') {
    try { conversationId = parseConversationId(requested); } catch { invalidSelection = true; }
  }
  const create = useCallback(async () => {
    if (creatingRef.current || !modelConfigured) return;
    creatingRef.current = true;
    setCreating(true);
    setActionError(false);
    const startedAt = currentIdentity.current;
    const storageKey = browserStorageKey('companion-session-creation', 'companion', companionId);
    let creationKey: string;
    try {
      creationKey = window.sessionStorage.getItem(storageKey) || uuidv7();
      window.sessionStorage.setItem(storageKey, creationKey);
    } catch { creationKey = uuidv7(); }
    try {
      const created = await ipcBridge.companion.createCompanionSession.invoke({ companion_id: companionId, creation_key: creationKey });
      try { window.sessionStorage.removeItem(storageKey); } catch { /* A confirmed creation needs no retry key. */ }
      refresh();
      if (mounted.current && currentIdentity.current === startedAt) select(created.conversation_id);
    } catch {
      if (mounted.current && currentIdentity.current === startedAt) setActionError(true);
    } finally {
      creatingRef.current = false;
      if (mounted.current) setCreating(false);
    }
  }, [companionId, modelConfigured, refresh, select]);
  const continueInWindow = useCallback(async () => {
    if (!conversationId || routing) return;
    setRouting(true);
    setActionError(false);
    const startedAt = currentIdentity.current;
    try {
      await ipcBridge.companion.continueInCompanionWindow.invoke({ companion_id: companionId, conversation_id: conversationId });
      refresh();
    } catch {
      if (mounted.current && currentIdentity.current === startedAt) setActionError(true);
    } finally { if (mounted.current) setRouting(false); }
  }, [companionId, conversationId, routing, refresh]);
  const manage = useCallback(async (id: ConversationId, command: () => Promise<unknown>, deleted = false) => {
    if (managementPending.current) throw new Error('A Session action is already pending');
    managementPending.current = true;
    setManaging(true);
    setActionError(false);
    const startedAt = currentIdentity.current;
    try {
      await command();
      if (deleted && mounted.current && currentIdentity.current === startedAt && conversationId === id) {
        setParams(previous => {
          const next = new URLSearchParams(previous);
          next.set('session', 'new');
          return next;
        }, { replace: true });
      }
      if (!deleted) void refreshConversationCache(id).catch(() => {});
      refresh();
    } catch (error) {
      refresh();
      if (mounted.current && currentIdentity.current === startedAt) setActionError(true);
      throw error;
    } finally {
      managementPending.current = false;
      if (mounted.current) setManaging(false);
    }
  }, [conversationId, refresh, setParams]);
  const rename = useCallback((id: ConversationId, name: string) => manage(id, async () => {
    if (!name.trim()) throw new Error('A Session name is required');
    const saved = await ipcBridge.conversation.update.invoke({ conversation_id: id, updates: { name: name.trim() } });
    if (!saved) throw new Error('Session rename was not committed');
  }), [manage]);
  const setArchived = useCallback((id: ConversationId, archived: boolean) => manage(id, async () => {
    const saved = await ipcBridge.conversation.update.invoke({ conversation_id: id, updates: { archived } });
    if (!saved) throw new Error('Session archive annotation was not committed');
  }), [manage]);
  const remove = useCallback((id: ConversationId) => manage(id,
    () => ipcBridge.conversation.remove.invoke({ conversation_id: id }), true), [manage]);
  const clearContext = useCallback((id: ConversationId, key: string) => manage(id,
    () => ipcBridge.conversation.clearContext.invoke({ conversation_id: id, idempotency_key: key })), [manage]);
  return { conversationId, items: list.data?.items ?? [], listError: Boolean(list.error),
    missing: Boolean(conversationId && list.data && !list.data.has_more && !list.isValidating
      && !list.data.items.some(item => item.conversation_id === conversationId)),
    error: bootstrapError || Boolean(target.error) || invalidSelection, actionError,
    activeDesktopId: target.data?.conversation_id ?? null, creating, routing, managing, selectionCleared: requested === 'new',
    select, create, continueInWindow, refresh, rename, setArchived, remove, clearContext };
}
