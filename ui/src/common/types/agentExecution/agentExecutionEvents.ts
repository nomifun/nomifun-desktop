import type { AgentExecutionEventKind } from '@/common/protocolBindings/AgentExecutionEventKind';
import type { ExecutionId } from '@/common/types/ids';

export type TAgentExecutionChangedEvent = {
  execution_id: ExecutionId;
  sequence: number;
  change_kind: AgentExecutionEventKind;
};

export type TAgentExecutionLeadThinkingPhase = 'planning' | 'adjust';
export type TAgentExecutionLeadThinkingKind = 'reasoning' | 'text';

export type TAgentExecutionLeadThinkingEvent = {
  execution_id: ExecutionId;
  phase: TAgentExecutionLeadThinkingPhase;
  kind: TAgentExecutionLeadThinkingKind;
  delta?: string;
  content?: string;
  done: boolean;
};
