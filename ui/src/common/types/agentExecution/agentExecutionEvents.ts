import type { AgentExecutionEventKind } from '@/common/protocolBindings/AgentExecutionEventKind';

export type TAgentExecutionChangedEvent = {
  execution_id: string;
  sequence: number;
  change_kind: AgentExecutionEventKind;
};

export type TAgentExecutionLeadThinkingPhase = 'planning' | 'adjust';
export type TAgentExecutionLeadThinkingKind = 'reasoning' | 'text';

export type TAgentExecutionLeadThinkingEvent = {
  execution_id: string;
  phase: TAgentExecutionLeadThinkingPhase;
  kind: TAgentExecutionLeadThinkingKind;
  delta?: string;
  content?: string;
  done: boolean;
};
