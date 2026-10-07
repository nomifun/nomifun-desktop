import type { TChatConversation } from '@/common/config/storage';
import type { TAgentExecutionDetail, TExecutionAttempt, TExecutionStep } from '@/common/types/agentExecution/agentExecutionTypes';
import { parseConversationId, parseExecutionAttemptId, parseExecutionId, parseExecutionStepId, parseMessageId, parseProviderId } from '@/common/types/ids';

export const leadConversationId = parseConversationId('00000000-03e8-7000-8000-000000000001');
export const requestId = parseMessageId('00000000-03e8-7000-8000-000000000002');
export const executionId = parseExecutionId('00000000-07d0-7000-8000-000000000003');
export const childConversationId = parseConversationId('00000000-09c4-7000-8000-000000000004');

export const makeStep = (index: number, overrides: Partial<TExecutionStep> = {}): TExecutionStep => ({
  step_id: parseExecutionStepId(`00000000-07d0-7000-8000-${String(index + 10).padStart(12, '0')}`),
  execution_id: executionId, title: `Task ${index}`, spec: 'Build and verify the requested game.',
  profile: null, kind: 'agent', agent_mode: 'normal', status: 'running', tool_policy: 'full', role: null,
  fanout_group: null, control_policy: null, failure_policy: 'fail_execution', assigned_participant_id: null,
  assignment_source: null, assignment_score: null, assignment_rationale: null, assignment_locked: false,
  preset_prompt: null, graph_x: null, graph_y: null, dispatch_after: null, introduced_in_revision: 1,
  superseded_in_revision: null, version: 1, created_at: 2000, updated_at: 3000, ...overrides,
});

export const makeAttempt = (step: TExecutionStep, overrides: Partial<TExecutionAttempt> = {}): TExecutionAttempt => ({
  attempt_id: parseExecutionAttemptId('00000000-09c4-7000-8000-000000000030'), execution_id: executionId,
  step_id: step.step_id, attempt_no: 1, participant_id: null, conversation_id: childConversationId,
  status: 'running', trigger_reason: 'initial', effective_config: {}, question: null, error: null,
  output_summary: null, output_files: [], tokens: null, retry_after: null, runtime_state: null,
  started_at: 2500, finished_at: null, version: 1, created_at: 2500, updated_at: 3000, ...overrides,
});

export const makeDetail = (overrides: Partial<TAgentExecutionDetail> = {}): TAgentExecutionDetail => ({
  execution: {
    execution_id: executionId, lead_conversation_id: leadConversationId, goal: 'Design and build a flight game',
    work_dir: null, delegation_policy: 'prefer_parallel', adaptation_policy: 'adaptive', decision_policy: 'automatic',
    max_parallel: 3, status: 'running', summary: null, version: 1, plan_revision: 1,
    event_sequence: 1, created_at: 2000, updated_at: 3000,
  },
  participants: [], steps: [], attempts: [], dependencies: [], ...overrides,
});

export const leadConversation: TChatConversation = {
  id: leadConversationId, type: 'nomi', name: 'Flight game', created_at: 1000, modified_at: 3000,
  extra: { workspace: '/delegation-test' }, linked_execution_id: executionId,
  model: { id: parseProviderId('00000000-03e8-7000-8000-000000000005'), platform: 'openai',
    name: 'Test provider', base_url: 'https://example.test/v1', auth_scheme: 'bearer', has_credentials: false, use_model: 'test-model' },
};
