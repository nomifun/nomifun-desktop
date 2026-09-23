/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/** Compare workspace identities carried by file events and conversation state. */
export function sameWorkspaceForRefresh(left: string, right: string): boolean {
  const normalize = (value: string): string => {
    const portable = value.replace(/\\/g, '/').replace(/\/+$/, '');
    return /^[a-zA-Z]:\//.test(portable) ? portable.toLowerCase() : portable;
  };
  return Boolean(left && right) && normalize(left) === normalize(right);
}

type Unsubscribe = () => void;

export interface ConversationWorkspaceRefreshSources {
  responseStream: (listener: (event: { conversation_id?: string; type: string; data?: unknown }) => void) => Unsubscribe;
  fileUpdates: (listener: (event: { workspace: string }) => void) => Unsubscribe;
  turnCompleted: (listener: (event: { conversation_id: string }) => void) => Unsubscribe;
  manual: (listener: () => void) => Unsubscribe;
}

/** Keep refresh ownership with one conversation subscription, including its timer. */
export function subscribeConversationWorkspaceRefresh(
  sources: ConversationWorkspaceRefreshSources,
  conversationId: string,
  workspace: string,
  refresh: () => void
): Unsubscribe {
  let timer: ReturnType<typeof setTimeout> | null = null;
  let pending = false;
  let active = true;
  const throttledRefresh = () => {
    if (!active) return;
    if (timer !== null) {
      pending = true;
      return;
    }
    refresh();
    timer = setTimeout(() => {
      timer = null;
      if (active && pending) {
        pending = false;
        refresh();
      }
    }, 2000);
  };
  const unsubscribers = [
    sources.responseStream((event) => {
      if (event.conversation_id && event.conversation_id !== conversationId) return;
      if (event.type === 'tool_call' && (event.data as { status?: string } | undefined)?.status === 'completed') {
        throttledRefresh();
      }
    }),
    sources.fileUpdates((event) => {
      if (sameWorkspaceForRefresh(event.workspace, workspace)) throttledRefresh();
    }),
    sources.turnCompleted((event) => {
      if (event.conversation_id === conversationId) throttledRefresh();
    }),
    sources.manual(refresh),
  ];
  return () => {
    active = false;
    pending = false;
    if (timer !== null) clearTimeout(timer);
    for (const unsubscribe of unsubscribers) unsubscribe();
  };
}
