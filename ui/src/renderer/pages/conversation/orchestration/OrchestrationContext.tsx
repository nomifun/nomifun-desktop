/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { TChatConversation } from '@/common/config/storage';
import type { TRunDetail } from '@/common/types/orchestrator/orchestratorTypes';
import type { OpenTaskPayload } from '@/renderer/pages/orchestrator/RunDetail/DagCanvas';
import type { LeadThinkingState } from '@/renderer/pages/orchestrator/useLeadThinking';
import React, { createContext, useCallback, useContext, useEffect, useMemo, useState } from 'react';
import { useConversationRun } from './useConversationRun';

/**
 * The single source of truth for 「会话原生编排 v2」per-conversation state.
 *
 * Composes the live run state from {@link useConversationRun} (F1) with the
 * conversation-local projection / floating-canvas UI state. Downstream features
 * — F5 (右栏 tab)、F6 (悬浮画布)、F7 (内容区投射)、F8 (头部入口 pill) — all read
 * this one value via {@link useOrchestration} instead of prop-drilling, so the
 * run id / detail / projection / canvas flags stay in lockstep.
 *
 * `projectedTaskId` / `projectedPayload` describe which DAG task (if any) the
 * content area is currently projecting to (default `null` = main conversation);
 * `projectedPayload` is cached so F7 can resolve the worker conversation_id off
 * the assignment without re-fetching. `canvasOpen` is the floating DAG canvas'
 * expand/collapse flag (default `false` = collapsed).
 */
export interface OrchestrationContextValue {
  conversationId: number;
  // run state (from useConversationRun — F1)
  runId: string | null;
  detail: TRunDetail | null;
  refetch: () => Promise<void>;
  leadThinking: LeadThinkingState;
  loading: boolean;
  // content-area projection
  projectedTaskId: string | null;
  projectedPayload: OpenTaskPayload | null; // cached so F7 can resolve the worker conversation_id
  projectTask: (payload: OpenTaskPayload) => void;
  returnToMain: () => void;
  // floating-canvas expand / collapse
  canvasOpen: boolean;
  openCanvas: () => void;
  collapseCanvas: () => void;
}

const OrchestrationContext = createContext<OrchestrationContextValue | null>(null);

/**
 * Provides the orchestration state for a single conversation. Wraps the
 * conversation panel's subtree so headerExtra / sider / chat body can all
 * consume the same run + projection + canvas state.
 *
 * Defaults are always "main": no task projected (`projectedTaskId === null`)
 * and the floating canvas collapsed (`canvasOpen === false`). When the linked
 * run changes or disappears, the projection is reset so we never project to a
 * node belonging to a stale run.
 */
export const OrchestrationProvider: React.FC<{ conversation: TChatConversation; children: React.ReactNode }> = ({
  conversation,
  children,
}) => {
  const conversationId = conversation.id;
  const { runId, detail, refetch, leadThinking, loading } = useConversationRun(conversation);

  const [projectedTaskId, setProjectedTaskId] = useState<string | null>(null);
  const [projectedPayload, setProjectedPayload] = useState<OpenTaskPayload | null>(null);
  const [canvasOpen, setCanvasOpen] = useState(false);

  const projectTask = useCallback((payload: OpenTaskPayload) => {
    setProjectedTaskId(payload.task.id);
    setProjectedPayload(payload);
  }, []);

  const returnToMain = useCallback(() => {
    setProjectedTaskId(null);
    setProjectedPayload(null);
  }, []);

  const openCanvas = useCallback(() => {
    setCanvasOpen(true);
  }, []);

  const collapseCanvas = useCallback(() => {
    setCanvasOpen(false);
  }, []);

  // Reset the projection whenever the linked run changes or disappears, so we
  // never keep projecting to a task belonging to a previous run. Depends only
  // on `runId` (a primitive) — the projection is local UI state, so clearing it
  // is the right behavior on any run-link change.
  useEffect(() => {
    setProjectedTaskId(null);
    setProjectedPayload(null);
  }, [runId]);

  // Auto-open the floating canvas on landing when this conversation was just
  // launched from the homepage「智能编排」entry (which stashes a per-conversation
  // sessionStorage flag). Wait until `runId` lights up so the overlay shows the
  // live canvas rather than briefly flashing the「发起」initiation state, then
  // consume the flag once so it never re-opens after the user collapses it.
  useEffect(() => {
    if (runId == null) return;
    const key = `nomi_open_canvas_${conversationId}`;
    let flagged = false;
    try {
      flagged = sessionStorage.getItem(key) != null;
      if (flagged) sessionStorage.removeItem(key);
    } catch {
      /* sessionStorage may be unavailable — non-fatal */
    }
    if (flagged) setCanvasOpen(true);
  }, [runId, conversationId]);

  const value = useMemo<OrchestrationContextValue>(
    () => ({
      conversationId,
      runId,
      detail,
      refetch,
      leadThinking,
      loading,
      projectedTaskId,
      projectedPayload,
      projectTask,
      returnToMain,
      canvasOpen,
      openCanvas,
      collapseCanvas,
    }),
    [
      conversationId,
      runId,
      detail,
      refetch,
      leadThinking,
      loading,
      projectedTaskId,
      projectedPayload,
      projectTask,
      returnToMain,
      canvasOpen,
      openCanvas,
      collapseCanvas,
    ]
  );

  return <OrchestrationContext.Provider value={value}>{children}</OrchestrationContext.Provider>;
};

/**
 * Read the orchestration state. Throws when called outside an
 * {@link OrchestrationProvider} — use {@link useOrchestrationSafe} from a
 * subtree that may render outside the provider.
 */
export function useOrchestration(): OrchestrationContextValue {
  const ctx = useContext(OrchestrationContext);
  if (ctx === null) {
    throw new Error('useOrchestration must be used within an <OrchestrationProvider>');
  }
  return ctx;
}

/**
 * Read the orchestration state, returning `null` outside an
 * {@link OrchestrationProvider} instead of throwing. For optional consumers
 * that may render on both orchestration and non-orchestration surfaces.
 */
export function useOrchestrationSafe(): OrchestrationContextValue | null {
  return useContext(OrchestrationContext);
}
