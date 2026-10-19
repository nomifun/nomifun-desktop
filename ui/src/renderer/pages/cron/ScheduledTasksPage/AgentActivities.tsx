import React, { useCallback, useEffect, useRef, useState } from 'react';
import { Button, Dropdown, Empty, Menu, Message, Spin, Tag } from '@arco-design/web-react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import { agentActivities } from '@/common/adapter/ipcBridge';
import type { AgentActivity, AgentActivityTarget, StopAgentActivityRequest } from '@/common/types/agentExecution/agentActivity';
import { uuidv7 } from '@/common/utils/uuidv7';

/** Polling reads existing owner facts only. Commands retain the exact target
 * captured at the click, including on a retry after a lost HTTP response. */
const AgentActivities: React.FC = () => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [items, setItems] = useState<AgentActivity[]>([]);
  const [cursor, setCursor] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string>();
  const [pending, setPending] = useState<Set<string>>(() => new Set());
  const [accepted, setAccepted] = useState<Set<string>>(() => new Set());
  const [capturing, setCapturing] = useState(false);
  const [batch, setBatch] = useState<{ at: number; items: Array<{ target: AgentActivityTarget; state: 'pending' | 'accepted' | 'unconfirmed'; error?: string; executionStopped?: boolean }> }>();
  const keys = useRef(new Map<string, string>());
  const pages = useRef(1);
  const alive = useRef(true);
  const reading = useRef(false);
  const refresh = useCallback(async () => {
    if (reading.current) return;
    reading.current = true;
    try {
      const found: AgentActivity[] = [];
      let after: string | undefined;
      for (let page = 0; page < pages.current; page += 1) {
        const response = await agentActivities.list.invoke({ after_cursor: after, limit: 50 });
        found.push(...response.items);
        after = response.next_cursor ?? undefined;
        if (!after) break;
      }
      if (alive.current) { setItems(found); setCursor(after ?? null); setError(undefined); }
    } catch (reason) {
      if (alive.current) setError(String(reason));
    } finally { reading.current = false; if (alive.current) setLoading(false); }
  }, []);
  useEffect(() => {
    alive.current = true;
    void refresh();
    const timer = setInterval(() => { void refresh(); }, 2000);
    return () => { alive.current = false; clearInterval(timer); };
  }, [refresh]);

  const stop = useCallback(async (target: AgentActivityTarget, retryCleanup = false, scope?: StopAgentActivityRequest['scope']) => {
    const identity = JSON.stringify({ target, retryCleanup, scope });
    const key = keys.current.get(identity) ?? uuidv7();
    keys.current.set(identity, key);
    setPending((current) => new Set(current).add(identity));
    try {
      const receipt = await agentActivities.stop.invoke({ target, idempotency_key: key, retry_cleanup: retryCleanup, ...(scope ? { scope } : {}) });
      if (receipt.accepted && alive.current && scope !== 'delegated_work') setAccepted((current) => new Set(current).add(JSON.stringify(target)));
      if (receipt.accepted && alive.current && scope === 'delegated_work') Message.success(t('cron.activity.delegatedAccepted'));
      await refresh();
      return { accepted: receipt.accepted, executionStopped: receipt.execution_stopped };
    } catch (reason) { if (alive.current) Message.error(String(reason)); return { accepted: false, error: String(reason), executionStopped: false }; }
    finally { if (alive.current) setPending((current) => { const next = new Set(current); next.delete(identity); return next; }); }
  }, [refresh, t]);
  const stopBatchItem = async (target: AgentActivityTarget) => {
    const result = await stop(target);
    if (alive.current) setBatch((current) => current ? { ...current, items: current.items.map((item) =>
      JSON.stringify(item.target) === JSON.stringify(target) ? { ...item, state: result.accepted ? 'accepted' : 'unconfirmed', error: result.error, executionStopped: result.executionStopped } : item) } : current);
  };
  const stopAll = async () => {
    setCapturing(true);
    try {
      // Reading the finite snapshot does not issue Stop. A lost read response
      // therefore cannot recapture a successor as a retry of an earlier batch.
      const snapshot = await agentActivities.stopTargets.invoke();
      setBatch({ at: snapshot.captured_at_ms, items: snapshot.targets.map((target) => ({ target, state: 'pending' })) });
      // Independent owners receive their exact commands concurrently.
      for (const target of snapshot.targets) void stopBatchItem(target);
    } catch (reason) { Message.error(String(reason)); }
    finally { if (alive.current) setCapturing(false); }
  };
  return <section className='flex flex-col gap-16px' aria-label={t('cron.activity.title')}>
    <div className='flex items-center justify-between gap-16px'>
      <p className='m-0 text-13px leading-20px text-t-secondary'>{t('cron.activity.description')}</p>
      <div className='flex shrink-0 gap-8px'>
        <Button onClick={() => { void refresh(); }}>{t('cron.activity.refresh')}</Button>
        <Button status='danger' loading={capturing} onClick={() => { void stopAll(); }}>{t('cron.activity.stopAll')}</Button>
      </div>
    </div>
    {error && <div role='alert' className='min-w-0 break-words text-13px text-t-secondary'>{error}</div>}
    {batch && <div role='status' className='rounded-12px border border-solid border-[var(--color-border-2)] p-12px text-13px text-t-secondary'>
      <div>{t('cron.activity.batchCutoff', { time: new Date(batch.at).toLocaleTimeString() })}</div>
      <div>{t('cron.activity.batchProgress', { accepted: batch.items.filter((item) => item.state === 'accepted').length, pending: batch.items.filter((item) => item.state === 'pending').length, unconfirmed: batch.items.filter((item) => item.state === 'unconfirmed').length })}</div>
      <div>{t('cron.activity.batchStopped', { count: batch.items.filter((item) => item.executionStopped).length })}</div>
      {batch.items.filter((item) => item.state === 'unconfirmed').map((item) => <div key={JSON.stringify(item.target)} className='mt-8px flex items-center justify-between gap-8px'>
        <span className='min-w-0 flex-1 break-words'>{item.error}</span><Button size='small' className='shrink-0' onClick={() => { void stopBatchItem(item.target); }}>{t('cron.activity.retryStop')}</Button>
      </div>)}
    </div>}
    {loading ? <Spin /> : items.length === 0 ? <Empty description={t(cursor ? 'cron.activity.emptyPage' : 'cron.activity.empty')} /> : items.map((item) => {
      const wasAccepted = item.stop_accepted || accepted.has(JSON.stringify(item.target));
      const busy = pending.has(JSON.stringify({ target: item.target, retryCleanup: false }));
      return <article key={item.key} className='rounded-12px border border-solid border-[var(--color-border-2)] p-16px'>
        <div className='flex flex-wrap items-start justify-between gap-16px'>
          <div className='min-w-0 flex-1 basis-200px'>
            <div className='break-words font-medium text-t-primary'>{item.title || t('cron.activity.unnamed')}</div>
            <div className='mt-8px flex flex-wrap items-center gap-8px text-12px text-t-secondary'>
              <span>{t(`cron.activity.kinds.${item.kind}`)}</span><Tag>{t(`cron.activity.states.${item.state}`)}</Tag>
              {wasAccepted && <span>{t('cron.activity.accepted')}</span>}
              {item.execution_stopped && <span>{t('cron.activity.stopped')}</span>}
            </div>
          </div>
          <div className='flex max-w-full flex-wrap justify-end gap-8px'>
            {item.agent_session_id && <Button size='small' onClick={() => navigate(`/conversation/${item.agent_session_id}`)}>{t('cron.activity.open')}</Button>}
            {item.can_stop && <Button size='small' status='danger' loading={busy} onClick={() => { void stop(item.target); }}>{t(item.target.kind === 'auto_work_run' ? 'cron.activity.stopAutomaticRun' : item.target.kind === 'execution_group' ? 'cron.activity.stopCollaboration' : item.kind === 'resources' ? item.target.kind === 'session_resources' ? 'cron.activity.retry' : 'cron.activity.closeResource' : 'cron.activity.stop')}</Button>}
            {item.target.kind === 'auto_work_run' && item.delegation_target && <Button size='small'
              loading={pending.has(JSON.stringify({ target: item.delegation_target, retryCleanup: false, scope: 'delegated_work' }))}
              onClick={() => { void stop(item.delegation_target!, false, 'delegated_work'); }}>{t('cron.activity.stopDelegated')}</Button>}
            {(item.target.kind === 'execution_task' || item.target.kind === 'execution_step') && <Dropdown trigger='click' position='br' droplist={<Menu>
              <Menu.Item key='delegated_work' onClick={() => { void stop(item.target, false, 'delegated_work'); }}>{t('cron.activity.stopDelegated')}</Menu.Item>
              <Menu.Item key='task_and_delegated' onClick={() => { void stop(item.target, false, 'task_and_delegated'); }}>{t('cron.activity.stopWithDelegated')}</Menu.Item>
            </Menu>}><Button size='small'>{t('cron.activity.stopScope')}</Button></Dropdown>}
            {item.can_retry_cleanup && !(item.can_stop && item.target.kind === 'session_resources') && <Button size='small' onClick={() => { void stop(item.target, true); }}>{t('cron.activity.retry')}</Button>}
          </div>
        </div>
        {item.continuation !== 'none' && <p className='mb-0 text-13px text-t-secondary'>{t(item.continuation === 'answer_queued' ? 'cron.activity.answerQueued' : 'cron.activity.resumeQueued')}</p>}
        {item.waiting_reason && <p className='mb-0 text-13px text-t-secondary'>{item.waiting_reason}</p>}
        <div className='mt-12px grid grid-cols-2 gap-x-24px gap-y-8px text-12px text-t-secondary'>
          {item.resources.map((resource) => <div key={JSON.stringify(resource.target ?? resource.kind)} className='flex min-w-0 items-start justify-between gap-8px'>
            <span className='min-w-0 flex-1 break-words'>{resource.display_name ?? t(`cron.activity.resources.${resource.kind}`)} · {t(`cron.activity.resourceStates.${resource.state}`)}</span>
            {resource.target && resource.state !== 'released' && <Button size='mini' type='text' className='shrink-0' onClick={() => { void stop(resource.target!); }}>{t('cron.activity.closeResource')}</Button>}
          </div>)}
        </div>
        {wasAccepted && item.resources.some((resource) => resource.state === 'unconfirmed') && <p className='mb-0 text-12px text-t-tertiary'>{t('cron.activity.releaseUnknown')}</p>}
      </article>;
    })}
    {cursor && <Button onClick={() => { pages.current += 1; void refresh(); }}>{t('cron.activity.more')}</Button>}
  </section>;
};

export default AgentActivities;
