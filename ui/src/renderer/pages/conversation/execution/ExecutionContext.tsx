import type { ConversationId, ExecutionId, ExecutionStepId } from '@/common/types/ids';
import type { TChatConversation } from '@/common/config/storage';
import type { TAgentExecutionDetail } from '@/common/types/agentExecution/agentExecutionTypes';
import React, { createContext, useCallback, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { OpenStepPayload } from './DagCanvas';
import type { LeadThinkingState } from './useLeadThinking';
import { useConversationExecution } from './useConversationExecution';

export interface ExecutionContextValue {
  conversationId: ConversationId;
  executionId: ExecutionId | null;
  detail: TAgentExecutionDetail | null;
  refetch: () => Promise<void>;
  leadThinking: LeadThinkingState;
  loading: boolean;
  projectedStepId: ExecutionStepId | null;
  projectedPayload: OpenStepPayload | null;
  projectStep: (payload: OpenStepPayload) => void;
  projectStepAsync: (load: () => Promise<OpenStepPayload | null>) => Promise<void>;
  returnToMain: () => void;
  canvasOpen: boolean;
  setCanvasOpen: (open: boolean) => void;
  toggleCanvas: () => void;
}

const ExecutionContext = createContext<ExecutionContextValue | null>(null);

export const ExecutionProvider: React.FC<{
  conversation: TChatConversation;
  children: React.ReactNode;
}> = ({ conversation, children }) => {
  const { executionId, detail, refetch, leadThinking, loading } = useConversationExecution(conversation);
  const [projectedPayload, setProjectedPayload] = useState<OpenStepPayload | null>(null);
  const [canvasOpen, setCanvasOpen] = useState(false);
  const projectionIntent = useRef(0);
  const mounted = useRef(false);
  const session = useRef(conversation.id);

  const projectStep = useCallback((payload: OpenStepPayload) => {
    if (!mounted.current || session.current !== conversation.id) return;
    projectionIntent.current += 1;
    setProjectedPayload(payload);
  }, [conversation.id]);
  const projectStepAsync = useCallback(async (load: () => Promise<OpenStepPayload | null>) => {
    if (!mounted.current || session.current !== conversation.id) return;
    const intent = ++projectionIntent.current;
    const current = () => mounted.current && session.current === conversation.id && intent === projectionIntent.current;
    try {
      const payload = await load();
      if (payload && current()) setProjectedPayload(payload);
    } catch (error) {
      if (current()) throw error;
    }
  }, [conversation.id]);
  const returnToMain = useCallback(() => {
    if (!mounted.current || session.current !== conversation.id) return;
    projectionIntent.current += 1;
    setProjectedPayload(null);
  }, [conversation.id]);
  const toggleCanvas = useCallback(() => setCanvasOpen((open) => !open), []);

  // Only committed UI navigation owns this fence. It is not an execution ledger.
  useLayoutEffect(() => {
    mounted.current = true;
    session.current = conversation.id;
    projectionIntent.current += 1;
    setProjectedPayload(null);
    return () => { mounted.current = false; projectionIntent.current += 1; };
  }, [conversation.id]);

  // A task opened from older history owns its exact execution identity. A new
  // latest execution may refresh the canvas without closing that transcript.
  useEffect(() => {
    setCanvasOpen(Boolean(executionId));
  }, [executionId]);

  const value = useMemo<ExecutionContextValue>(
    () => ({
      conversationId: conversation.id,
      executionId,
      detail,
      refetch,
      leadThinking,
      loading,
      projectedStepId: projectedPayload?.step.step_id ?? null,
      projectedPayload,
      projectStep,
      projectStepAsync,
      returnToMain,
      canvasOpen,
      setCanvasOpen,
      toggleCanvas,
    }),
    [
      canvasOpen,
      conversation.id,
      detail,
      executionId,
      leadThinking,
      loading,
      projectStep,
      projectStepAsync,
      projectedPayload,
      refetch,
      returnToMain,
      toggleCanvas,
    ],
  );

  return <ExecutionContext.Provider value={value}>{children}</ExecutionContext.Provider>;
};

export function useExecution(): ExecutionContextValue {
  const context = useContext(ExecutionContext);
  if (!context) throw new Error('useExecution must be used within an <ExecutionProvider>');
  return context;
}

export function useExecutionSafe(): ExecutionContextValue | null {
  return useContext(ExecutionContext);
}
