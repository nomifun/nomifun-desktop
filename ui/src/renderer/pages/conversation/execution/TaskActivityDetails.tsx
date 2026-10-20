import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { agentActivities, agentExecution, conversation } from '@/common/adapter/ipcBridge';
import type { AgentActivity, AgentActivityTarget } from '@/common/types/agentExecution/agentActivity';
import type { ExecutionAttemptId, ExecutionId, ExecutionStepId } from '@/common/types/ids';
import { uuidv7 } from '@/common/utils/uuidv7';

/** Shared owner-fact presentation for the task inspector and activity list.
 * An accepted command, stopped execution and released resources stay distinct. */
export function ActivityCleanupProgress({ item, accepted = item.stop_accepted, executionStopped = item.execution_stopped,
  showStatus = true, onCloseResource }: {
  item: AgentActivity; accepted?: boolean; executionStopped?: boolean; showStatus?: boolean;
  onCloseResource?: (target: AgentActivityTarget) => void;
}) {
  const { t } = useTranslation();
  return <div className='mt-8px text-12px text-t-secondary' data-activity-cleanup-progress>
    {showStatus && <div className='mb-8px flex flex-wrap gap-8px' role='status'>
      <span>{t(`cron.activity.states.${item.state}`)}</span>
      {accepted && <span>{t('cron.activity.accepted')}</span>}
      {executionStopped ? <span>{t('cron.activity.stopped')}</span>
        : accepted && <span>{t('agentExecution.activity.stopPending')}</span>}
    </div>}
    <div className='flex flex-col gap-6px'>
      {item.resources.map((resource, index) => <div key={`${resource.kind}:${index}`} className='min-w-0'>
        <div className='flex items-start justify-between gap-8px'>
          <span className='min-w-0 flex-1 break-words'>{resource.display_name ?? t(`cron.activity.resources.${resource.kind}`)} · {t(`cron.activity.resourceStates.${resource.state}`)}</span>
          {onCloseResource && resource.target && resource.state !== 'released' && <button type='button'
            className='shrink-0 border-0 bg-transparent p-0 text-inherit cursor-pointer'
            onClick={() => onCloseResource(resource.target!)}>{t('cron.activity.closeResource')}</button>}
        </div>
        {resource.detail && <p className='m-0 break-words text-t-tertiary'>{resource.detail}</p>}
      </div>)}
    </div>
    {accepted && item.resources.some(resource => resource.state === 'unconfirmed') && <p className='mb-0'>{t('cron.activity.releaseUnknown')}</p>}
  </div>;
}

/** Observes only the selected exact Attempt while this inspector is mounted.
 * No global activity walk, latest-Attempt fallback, runtime start or model input. */
export default function TaskActivityDetails({ executionId, stepId, attemptId }: {
  executionId: ExecutionId; stepId: ExecutionStepId; attemptId: ExecutionAttemptId;
}) {
  const { t } = useTranslation();
  const scope = `${executionId}:${stepId}:${attemptId}`;
  const current = useRef(scope); current.current = scope;
  const alive = useRef(true);
  const [snapshot, setSnapshot] = useState<{ scope: string; item: AgentActivity }>();
  const [failed, setFailed] = useState(false);
  const [loading, setLoading] = useState(true);
  const [revision, setRevision] = useState(0);
  const [retrying, setRetrying] = useState(false);
  const [commandFailed, setCommandFailed] = useState(false);
  const [receipt, setReceipt] = useState<{ scope: string; accepted: boolean; executionStopped: boolean }>();
  const mutationRunning = useRef(false);
  const retryCommand = useRef<{ scope: string; target: AgentActivityTarget; key: string } | undefined>(undefined);
  useEffect(() => {
    alive.current = true;
    return () => { alive.current = false; };
  }, []);
  useEffect(() => {
    let disposed = false;
    let reading = false;
    let pending = false;
    let settled = false;
    setLoading(true); setFailed(false);
    const refresh = async () => {
      if (reading) { pending = true; return; }
      reading = true;
      try {
        const response = await agentActivities.list.invoke({ execution_id: executionId, step_id: stepId, attempt_id: attemptId });
        if (disposed) return;
        const item = response.items[0];
        if (response.items.length !== 1 || response.next_cursor || item.target.kind !== 'execution_task'
          || item.target.execution_id !== executionId || item.target.step_id !== stepId || item.target.attempt_id !== attemptId)
          throw new Error('The activity source does not match the selected Attempt');
        setSnapshot({ scope, item });setFailed(false);
        settled = item.execution_stopped && !item.can_retry_cleanup && item.resources.length > 0
          && item.resources.every(resource => resource.state === 'released');
      } catch { if (!disposed) setFailed(true); }
      finally {
        reading = false;
        if (!disposed) setLoading(false);
        if (pending && !disposed) { pending = false; void refresh(); }
      }
    };
    void refresh();
    const offChanged = agentExecution.events.changed.on(event => { if (event.execution_id === executionId) void refresh(); });
    const offReconnect = conversation.reconnected.on(() => void refresh());
    const timer = window.setInterval(() => { if (!settled) void refresh(); }, 2_000);
    return () => { disposed = true; offChanged(); offReconnect(); window.clearInterval(timer); };
  }, [executionId, stepId, attemptId, scope, revision]);
  const item = snapshot?.scope === scope ? snapshot.item : undefined;
  const retryCleanup = async () => {
    if (!item?.can_retry_cleanup || mutationRunning.current) return;
    const command = retryCommand.current?.scope === scope ? retryCommand.current
      : { scope, target: structuredClone(item.target), key: uuidv7() };
    retryCommand.current = command;
    mutationRunning.current = true;setRetrying(true);setCommandFailed(false);
    try {
      const result = await agentActivities.stop.invoke({ target: command.target, idempotency_key: command.key, retry_cleanup: true });
      if (!result.accepted) throw new Error('Cleanup was not accepted');
      if (alive.current && current.current === command.scope) {
        retryCommand.current = undefined;
        setReceipt({ scope, accepted: result.accepted, executionStopped: result.execution_stopped });
        setRevision(value => value + 1);
      }
    } catch { if (alive.current && current.current === command.scope) setCommandFailed(true); }
    finally {
      mutationRunning.current = false;
      if (alive.current && current.current === command.scope) setRetrying(false);
    }
  };
  const observedReceipt = receipt?.scope === scope ? receipt : undefined;
  return <section aria-label={t('agentExecution.activity.title')} className='shrink-0 px-16px py-10px text-12px text-t-secondary'
    data-task-activity-attempt={attemptId}>
    <div className='flex items-center justify-between gap-8px'>
      <strong className='font-medium'>{t('agentExecution.activity.title')}</strong>
      <button type='button' className='border-0 bg-transparent p-0 text-inherit cursor-pointer' onClick={() => setRevision(value => value + 1)}>{t('cron.activity.refresh')}</button>
    </div>
    {loading && !item && <p role='status'>{t('agentExecution.activity.loading')}</p>}
    {failed && <p role='alert'>{t('agentExecution.activity.readFailed')}</p>}
    {item && <ActivityCleanupProgress item={item} accepted={item.stop_accepted || observedReceipt?.accepted}
      executionStopped={item.execution_stopped || observedReceipt?.executionStopped} />}
    {item?.can_retry_cleanup && <button type='button' className='mt-8px border-0 bg-transparent p-0 text-inherit cursor-pointer'
      disabled={retrying} onClick={() => void retryCleanup()}>{t('cron.activity.retry')}</button>}
    {commandFailed && <p role='alert'>{t('agentExecution.activity.cleanupFailed')}</p>}
  </section>;
}
