/** Read-only facts from original lifecycle owners. Targets are opaque command identities. */
export type AgentActivityTarget =
  | { kind: 'session_input'; agent_session_id: string; message_id: string }
  | { kind: 'session_turn'; agent_session_id: string; operation_id: string }
  | { kind: 'execution_task'; execution_id: string; step_id: string; attempt_id: string; expected_execution_version: number; expected_step_version: number }
  | { kind: 'execution_step'; execution_id: string; step_id: string; expected_execution_version: number; expected_step_version: number }
  | { kind: 'execution_group'; execution_id: string; expected_version: number }
  | { kind: 'cron_run'; cron_job_id: string; cron_job_run_id: string }
  | { kind: 'auto_work_run'; target_kind: 'conversation' | 'terminal'; target_id: string; requirement_id: string; claim_generation: number }
  | { kind: 'remote_session'; agent_session_id: string; operation_id: string | null; expected_head_seq: number; expected_binding_version: number }
  | { kind: 'runtime_carrier'; agent_session_id: string; instance_id: string; expected_generation: number | null; expected_head_seq: number }
  | { kind: 'browser_workspace'; agent_session_id: string; service_instance_id: string; runtime_generation: number; expected_head_seq: number }
  | { kind: 'session_resources'; agent_session_id: string; expected_head_seq: number }
  | { kind: 'knowledge_writeback'; agent_session_id: string; operation_id: string }
  | { kind: 'process_instance'; agent_session_id: string; owner_generation: string; process_id: string; creator_turn_id: string; creator_call_id: string }
  | { kind: 'ssh_link'; agent_session_id: string; ssh_host_id: string; pool_instance_id: string; instance_id: number };

export type AgentActivityState = 'queued' | 'preparing' | 'planning' | 'running' | 'waiting' | 'paused' | 'recovering' | 'stopping' | 'completed' | 'failed' | 'cancelled' | 'unconfirmed';
export type AgentActivityResourceState = 'active' | 'held_for_user' | 'releasing' | 'released' | 'unconfirmed' | 'unavailable';
export type AgentActivityResourceKind = 'runtime' | 'processes' | 'browser' | 'ssh' | 'knowledge_writeback' | 'effects';
export interface AgentActivityResource {
  kind: AgentActivityResourceKind;
  state: AgentActivityResourceState;
  detail: string | null;
  display_name?: string;
  target?: AgentActivityTarget;
}
export interface AgentActivity {
  key: string;
  title: string;
  kind: 'conversation' | 'companion' | 'delegated_task' | 'cron_run' | 'auto_work_run' | 'remote_session' | 'resources' | 'knowledge_writeback' | 'collaboration';
  state: AgentActivityState;
  agent_session_id: string | null;
  target: AgentActivityTarget;
  continuation: 'none' | 'answer_queued' | 'resume_queued';
  /** Only an AutoWork root exposes this separate, delegated-work-only target. */
  delegation_target?: Extract<AgentActivityTarget, { kind: 'execution_task' | 'execution_step' }> | null;
  waiting_reason: string | null;
  stop_accepted: boolean;
  execution_stopped: boolean;
  resources: AgentActivityResource[];
  can_stop: boolean;
  can_retry_cleanup: boolean;
  updated_at_ms: number | null;
}
export interface AgentActivitiesResponse { items: AgentActivity[]; next_cursor: string | null }
export interface AgentActivityStopSnapshot { captured_at_ms: number; targets: AgentActivityTarget[] }
export interface StopAgentActivityRequest { target: AgentActivityTarget; idempotency_key: string; retry_cleanup?: boolean; scope?: 'task' | 'delegated_work' | 'task_and_delegated' }
export interface AgentActivityCommandReceipt { accepted: boolean; execution_stopped: boolean; resources: AgentActivityResource[] }
