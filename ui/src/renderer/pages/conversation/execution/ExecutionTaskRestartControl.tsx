import { Redo } from '@icon-park/react';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ipcBridge } from '@/common';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import { latestAttemptForStep, type TAgentExecutionMessageReceipt, type TRestartExecutionStep } from '@/common/types/agentExecution/agentExecutionTypes';
import type { ExecutionAttemptId, ExecutionId, ExecutionStepId } from '@/common/types/ids';
import { uuidv7 } from '@/common/utils/uuidv7';
import styles from './projectedAttemptView.module.css';
import { useExecutionSafe } from './ExecutionContext';

type RestartState = { scope: string; phase: 'sending' | 'accepted' | 'unconfirmed' | 'blocked' | 'resolved'; text?: string; code?: string };
type RestartCommand = { scope: string; updates: TRestartExecutionStep; receipt?: TAgentExecutionMessageReceipt };

/** Read original command facts, including explicit resolutions. A complete GET
 * can prove a rejected draft was never accepted; a failed read cannot. */
async function readRestartCommands(executionId: ExecutionId, stepId: ExecutionStepId, attemptId: ExecutionAttemptId | null,
  active: () => boolean) {
  const requested = new Map<string, { receipt: TAgentExecutionMessageReceipt; key?: string }>();
  const resolved = new Set<string>();
  let after = 0;
  while (active()) {
    const events = await ipcBridge.agentExecution.listEvents.invoke({ execution_id: executionId,
      query: { after_sequence: after, limit: 200 } });
    if (!active()) return null;
    for (const event of events) {
      if (event.execution_id !== executionId || event.sequence <= after) throw new Error('Restart facts changed their source');
      after = event.sequence;
      if (event.step_id !== stepId || event.event_type !== 'step_changed' || !event.payload || typeof event.payload !== 'object') continue;
      const payload = event.payload as Record<string, unknown>;
      if (typeof payload.operation_id !== 'string') continue;
      if (payload.change === 'restart_resolved') { resolved.add(payload.operation_id); continue; }
      if (payload.change !== 'restart_requested' || event.attempt_id !== attemptId) continue;
      const command = payload.command_request as { request?: { idempotency_key?: unknown } } | undefined;
      requested.set(payload.operation_id, { receipt: { execution_id: executionId, step_ids: [stepId],
        sequence: event.sequence, operation_id: payload.operation_id, replayed: true },
        key: typeof command?.request?.idempotency_key === 'string' ? command.request.idempotency_key : undefined });
    }
    if (events.length < 200) return { requested, resolved, through: after };
  }
  return null;
}

/** Explicit user command followed through its original domain receipt. Reads
 * never resubmit it; a lost ACK retains the exact request for the next click. */
export default function ExecutionTaskRestartControl({ executionId, stepId, attemptId, executionVersion, stepVersion, refetch }: {
  executionId: ExecutionId; stepId: ExecutionStepId; attemptId: ExecutionAttemptId | null;
  executionVersion: number; stepVersion: number; refetch: () => Promise<void>;
}) {
  const { t } = useTranslation();
  const context = useExecutionSafe();
  const scope = `${executionId}:${stepId}:${attemptId ?? 'unstarted'}`;
  const current = useRef(scope); current.current = scope;
  const alive = useRef(true);
  const inFlight = useRef<RestartCommand | undefined>(undefined);
  const command = useRef<RestartCommand | undefined>(undefined);
  const [state, setState] = useState<RestartState>();
  const [receipt, setReceipt] = useState<{ scope: string; value: TAgentExecutionMessageReceipt }>();
  const observed = state?.scope === scope ? state : undefined;
  const accepted = receipt?.scope === scope ? receipt.value : undefined;
  const refresh = useRef(refetch); refresh.current = refetch;
  const [discoveredScope, setDiscoveredScope] = useState('');

  useEffect(() => {
    alive.current = true;
    return () => { alive.current = false; };
  }, []);
  useEffect(() => {
    if (command.current?.scope !== scope) command.current = undefined;
    for (const key of context?.restartDrafts.keys() ?? []) {
      if (key.startsWith(`${executionId}:${stepId}:`) && key !== scope) context?.restartDrafts.delete(key);
    }
  }, [executionId, stepId, scope, context?.restartDrafts]);

  useEffect(() => {
    let disposed = false;
    void readRestartCommands(executionId, stepId, attemptId, () => !disposed).then(facts => {
      if (disposed || !facts) return;
      const pending = [...facts.requested.values()].find(value => !facts.resolved.has(value.receipt.operation_id));
      if (pending) {
        setReceipt({ scope, value: pending.receipt }); setState({ scope, phase: 'accepted' });
        context?.restartDrafts.delete(scope);
      }
    }).catch(() => { /* Failed observation grants no mutation or new request identity. */ })
      .finally(() => { if (!disposed) setDiscoveredScope(scope); });
    return () => { disposed = true; };
  }, [executionId, stepId, attemptId, scope, context?.restartDrafts]);

  useEffect(() => {
    if (!accepted) return;
    let disposed = false, reading = false, again = false, resolved = false;
    let after = accepted.sequence - 1;
    const observe = async () => {
      if (disposed || resolved) return;
      if (reading) { again = true; return; }
      reading = true;
      try {
        // Follow only this command's original receipt. Pagination cannot jump
        // over a resolution among unrelated sibling progress events.
        for (let page = 0; page < 8 && !disposed; page++) {
          const events = await ipcBridge.agentExecution.listEvents.invoke({ execution_id: executionId,
            query: { after_sequence: after, limit: 200 } });
          if (disposed) return;
          for (const event of events) {
            if (event.execution_id !== executionId || event.sequence <= after) throw new Error('Restart observation changed its source');
            after = event.sequence;
            if (event.step_id !== stepId || event.event_type !== 'step_changed' || !event.payload || typeof event.payload !== 'object') continue;
            const value = event.payload as Record<string, unknown>;
            if (value.operation_id !== accepted.operation_id || value.change !== 'restart_resolved') continue;
            if (event.attempt_id !== attemptId || !['continued_original', 'retry_ready', 'original_pending', 'blocked', 'superseded'].includes(String(value.disposition))) {
              throw new Error('Restart resolution differs from its original Attempt or disposition');
            }
            resolved = true;
            if (command.current?.scope === scope && command.current.receipt?.operation_id === accepted.operation_id) command.current = undefined;
            context?.restartDrafts.delete(scope);
            setReceipt(undefined);
            setState({ scope, phase: value.disposition === 'blocked' ? 'blocked' : 'resolved',
              text: typeof value.reason === 'string' ? value.reason : undefined,
              code: typeof value.code === 'string' ? value.code : undefined });
            await refresh.current();
            return;
          }
          if (events.length < 200) break;
          if (page === 7) again = true;
        }
      } catch {
        // Acceptance is already durable. A failed GET cannot turn it back into
        // a new command or falsely announce that the task completed.
      } finally {
        reading = false;
        if (again && !disposed && !resolved) { again = false; queueMicrotask(() => void observe()); }
      }
    };
    void observe();
    const offChanged = ipcBridge.agentExecution.events.changed.on(event => { if (event.execution_id === executionId) void observe(); });
    const offReconnect = ipcBridge.conversation.reconnected.on(() => void observe());
    const timer = window.setInterval(() => void observe(), 2_000);
    return () => { disposed = true; offChanged(); offReconnect(); window.clearInterval(timer); };
  }, [accepted, executionId, stepId, attemptId, scope, context?.restartDrafts]);

  const restart = async () => {
    if (inFlight.current?.scope === scope || accepted || discoveredScope !== scope) return;
    const original = command.current?.scope === scope ? command.current : { scope, updates: context?.restartDrafts.get(scope) ?? {
      idempotency_key: uuidv7(), expected_execution_version: executionVersion,
      expected_step_version: stepVersion, expected_attempt_id: attemptId,
    } };
    command.current = original; inFlight.current = original;
    context?.restartDrafts.set(scope, original.updates);
    setState({ scope, phase: 'sending' });
    try {
      const result = await ipcBridge.agentExecution.restart.invoke({ execution_id: executionId, step_id: stepId, updates: original.updates });
      if (result.execution_id !== executionId || result.step_ids.length !== 1 || result.step_ids[0] !== stepId) {
        throw new Error('Restart acceptance differs from the selected task');
      }
      original.receipt = result;
      context?.restartDrafts.delete(scope);
      if (!alive.current || current.current !== scope) return;
      setReceipt({ scope, value: result }); setState({ scope, phase: 'accepted' });
      try { await refresh.current(); } catch { /* The accepted receipt stays authoritative. */ }
    } catch (error) {
      if (!alive.current || current.current !== scope) return;
      let rejected = false;
      if (isBackendHttpError(error) && error.status >= 400 && error.status < 500) {
        try {
          const fresh = await ipcBridge.agentExecution.get.invoke({ execution_id: executionId });
          const facts = await readRestartCommands(executionId, stepId, attemptId, () => alive.current && current.current === scope);
          if (!alive.current || current.current !== scope) return;
          const known = [...facts?.requested.values() ?? []].find(value => value.key === original.updates.idempotency_key);
          if (known) {
            original.receipt = known.receipt; context?.restartDrafts.delete(scope);
            setReceipt({ scope, value: known.receipt }); setState({ scope, phase: 'accepted' });
            return;
          }
          if (facts && fresh && facts.through >= fresh.execution.event_sequence) {
            rejected = fresh?.execution.execution_id === executionId
              && fresh.steps.some(value => value.step_id === stepId && value.superseded_in_revision == null)
              && (latestAttemptForStep(fresh.attempts, stepId)?.attempt_id ?? null) === attemptId;
          }
        } catch { /* Neither a failed GET nor a transport error proves rejection. */ }
      }
      if (rejected) { command.current = undefined; context?.restartDrafts.delete(scope); }
      setState({ scope, phase: rejected ? 'blocked' : 'unconfirmed', text: rejected ? String(error) : undefined });
      if (rejected) { try { await refresh.current(); } catch { /* Retain the original command error. */ } }
    } finally {
      if (inFlight.current === original) inFlight.current = undefined;
    }
  };
  const discovering = discoveredScope !== scope;
  const busy = discovering || observed?.phase === 'sending' || Boolean(accepted);
  return <div className='flex max-w-full flex-wrap items-center gap-8px'>
    <button type='button' disabled={busy} aria-disabled={busy} aria-busy={busy} className={styles.action}
      onClick={() => void restart()}>
      <Redo theme='outline' size='13' />
      {t(discovering ? 'agentExecution.restart.checking' : busy ? 'agentExecution.restart.pending' : 'agentExecution.restart.button')}
    </button>
    {observed && observed.phase !== 'sending' && <span role={observed.phase === 'blocked' || observed.phase === 'unconfirmed' ? 'alert' : 'status'}
      className='text-12px text-t-secondary' title={observed.text}>
      {t(observed.phase === 'accepted' ? 'agentExecution.restart.accepted' : observed.phase === 'unconfirmed'
        ? 'agentExecution.restart.unconfirmed' : observed.phase === 'blocked' ? 'agentExecution.restart.blocked' : 'agentExecution.restart.resolved')}
      {observed.phase === 'blocked' && (observed.code === 'USER_PAUSE_REQUIRES_CONTINUE' || observed.code === 'AGENT_NATIVE_RESUME_FORBIDDEN')
        && ` ${t('agentExecution.restart.continueRequired')}`}
      {observed.phase === 'blocked' && observed.code === 'NATIVE_RESTART_REQUIRES_RECONCILIATION'
        && ` ${t('agentExecution.restart.reconcileRequired')}`}
    </span>}
  </div>;
}
