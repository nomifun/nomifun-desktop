import type { ConversationId, ExecutionAttemptId, ExecutionId, ExecutionStepId, MessageId } from '../types/ids';

/** Source references are authored by the host; UI placement never uses wall-time guesses. */
export interface DelegationRecord {
  id: string;
  kind: 'delegated' | 'completed' | 'failed' | 'skipped';
  execution_id: ExecutionId;
  execution_event_sequence: number;
  step_id: ExecutionStepId;
  attempt_id: ExecutionAttemptId | null;
  title: string;
  source_scoped_order: boolean;
  source: {
    agent_session_id: ConversationId;
    turn_operation_id: string;
    source_message_id: MessageId;
    tool_operation_id: string;
    tool_call_id: string;
    effect_id: string;
    tool_event_id: string;
    tool_event_seq: number;
  };
  anchor: { agent_session_id: ConversationId; event_id: string; seq: number };
}
export interface DelegationRecordsPage {
  records: DelegationRecord[];
  through_session_seq: number;
  next_cursor?: string | null;
}
