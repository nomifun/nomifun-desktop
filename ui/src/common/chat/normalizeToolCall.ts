import type { IMessageToolCall, IMessageToolGroup } from './chatLib';
import { toDisplayText } from './displayText';
import { normalizeToolGroupStatus } from './toolGroupStatus';

export type NormalizedToolStatus = 'pending' | 'running' | 'completed' | 'error' | 'canceled';
export type NormalizedToolNotExecutedReason = 'invalid_arguments' | 'runtime_preflight' | 'process_reference';
export type NormalizedToolBoundedResult = 'search_context_withheld';

export interface NormalizedToolCall {
  key: string;
  name: string;
  capabilityId?: string;
  actionId?: string;
  status: NormalizedToolStatus;
  /** Tool reported an error-like outcome, but it should not fail the turn-level process receipt. */
  nonFatalFailure?: boolean;
  /** Exit status of a native command whose completion and cleanup are proven. */
  commandExitCode?: number;
  /** The local process owner proved that the requested executable did not start. */
  commandNotStarted?: boolean;
  /** A native deadline result with proven cleanup; the source error status remains intact. */
  commandTimedOut?: boolean;
  /** Exact local Runtime result whose data was intentionally withheld at a documented bound. */
  boundedResult?: NormalizedToolBoundedResult;
  /** Tool was not executed because an earlier call in the same assistant turn failed. */
  skipped?: boolean;
  /** The runtime rejected the call before dispatching it to the tool. */
  notExecutedReason?: NormalizedToolNotExecutedReason;
  description?: string;
  /** Bounded public diagnostic supplied by the canonical host. */
  error?: string;
}

const formatValue = (value: unknown): string => toDisplayText(value);

// ===== tool_group → NormalizedToolCall[] =====

function toNormalizedToolGroupStatus(status: unknown): NormalizedToolStatus {
  switch (normalizeToolGroupStatus(status)) {
    case 'Success':
      return 'completed';
    case 'Error':
      return 'error';
    case 'Canceled':
      return 'canceled';
    case 'Pending':
      return 'pending';
    case 'Executing':
    default:
      return 'running';
  }
}

export function normalizeToolGroup(message: IMessageToolGroup): NormalizedToolCall[] {
  if (!Array.isArray(message.content)) return [];
  return message.content.map(({ name, call_id, description, status }) => {
    const displayStatus = normalizeToolGroupStatus(status);
    const desc = typeof description === 'string' ? description : '';

    return {
      key: toDisplayText(call_id),
      name: toDisplayText(name, 'Tool'),
      status: toNormalizedToolGroupStatus(displayStatus),
      description: desc,
    };
  });
}

// Tool outcome metadata is authored by the host. Raw tool results never enter
// this display model and cannot be interpreted as status or public diagnostics.
function normalizeToolCallStatus(status?: unknown): NormalizedToolStatus {
  return status === 'completed' || status === 'error' || status === 'canceled' || status === 'running'
    ? status : 'pending';
}

export function normalizeToolCall(message: IMessageToolCall): NormalizedToolCall | undefined {
  const { call_id, name, capability_id, action_id, status, description, error } = message.content;
  if (!call_id) return undefined;
  const rawHold = message.content.runtime_hold;
  const hold = status === 'error' && rawHold && new Set(['tool_not_exposed', 'arguments_rejected', 'batch_rejected', 'phase_restricted', 'input_superseded', 'prior_call_failed', 'runtime_gate']).has(rawHold) ? rawHold : undefined;
  const skipped = hold === 'prior_call_failed';
  const invalidArgumentsNotExecuted = hold === 'arguments_rejected';
  const runtimePreflightNotExecuted = Boolean(hold) && !skipped && !invalidArgumentsNotExecuted;
  const terminal = status === 'completed' || status === 'error';
  const searchContextWithheld = status === 'error' && message.content.bounded_result === 'search_context_withheld';
  const processReferenceNotExecuted = status === 'error' && message.content.process_reference_invalid === true;
  const commandExitCode = terminal && Number.isSafeInteger(message.content.command_exit_code)
    && message.content.command_exit_code! >= 0 ? message.content.command_exit_code : undefined;
  const commandNotStarted = status === 'error' && message.content.command_not_started === true;
  const commandTimedOut = status === 'error' && message.content.command_timed_out === true;
  const nonFatalFailure = status === 'error' && message.content.non_fatal_failure === true;

  return {
    key: toDisplayText(call_id),
    name: toDisplayText(name, 'Tool'),
    status: skipped || invalidArgumentsNotExecuted || runtimePreflightNotExecuted || processReferenceNotExecuted
      ? 'canceled' : normalizeToolCallStatus(status),
    ...(typeof capability_id === 'string' && capability_id ? { capabilityId: capability_id } : {}),
    ...(typeof action_id === 'string' && action_id ? { actionId: action_id } : {}),
    ...(skipped ? { skipped: true } : {}),
    ...(invalidArgumentsNotExecuted ? { notExecutedReason: 'invalid_arguments' as const } : {}),
    ...(runtimePreflightNotExecuted ? { notExecutedReason: 'runtime_preflight' as const } : {}),
    ...(processReferenceNotExecuted ? { notExecutedReason: 'process_reference' as const } : {}),
    ...(searchContextWithheld ? { boundedResult: 'search_context_withheld' as const } : {}),
    ...(nonFatalFailure ? { nonFatalFailure: true } : {}),
    ...(commandExitCode !== undefined ? { commandExitCode } : {}),
    ...(commandNotStarted ? { commandNotStarted: true } : {}),
    ...(commandTimedOut ? { commandTimedOut: true } : {}),
    description: description ? formatValue(description) : undefined,
    ...(status === 'error' && error ? { error: formatValue(error) } : {}),
  };
}

// ===== Unified entry =====

export type ToolMessage = IMessageToolGroup | IMessageToolCall;

export function normalizeToolMessages(messages: ToolMessage[]): NormalizedToolCall[] {
  return messages
    .flatMap((m) => {
      if (m.type === 'tool_group') return normalizeToolGroup(m);
      if (m.type === 'tool_call') return normalizeToolCall(m);
      return undefined;
    })
    .filter((item): item is NormalizedToolCall => item !== undefined);
}
