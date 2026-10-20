import { useEffect, useId, useRef, useState } from 'react';
import { Right, ShareOne } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import { ipcBridge } from '@/common';
import type { DelegationRecord } from '@/common/chat/delegationRecords';
import { useExecutionSafe } from '../../execution/ExecutionContext';
import { latestAttemptForStep } from '@/common/types/agentExecution/agentExecutionTypes';

/** One operation summary; its original records retain exact task navigation. */
export default function DelegationRecordRow({ records }: { records: readonly DelegationRecord[] }) {
  const { t } = useTranslation();
  const execution = useExecutionSafe();
  const [expanded, setExpanded] = useState(false);
  const [openingId, setOpeningId] = useState<string>();
  const [failed, setFailed] = useState(false);
  const detailsId = useId();
  const active = useRef(true);
  useEffect(() => { active.current = true; return () => { active.current = false; }; }, []);
  const open = async (record: DelegationRecord) => {
    if (!execution || openingId) return;
    setOpeningId(record.id); setFailed(false);
    try {
      await execution.projectStepAsync(async () => {
        const snapshot = execution.detail?.execution.execution_id === record.execution_id ? execution.detail : null;
        const detail = snapshot && snapshot.steps.some(step => step.step_id === record.step_id)
          && (!record.attempt_id || snapshot.attempts.some(attempt => attempt.attempt_id === record.attempt_id && attempt.step_id === record.step_id))
          ? snapshot : await ipcBridge.agentExecution.get.invoke({ execution_id: record.execution_id });
        if (!active.current) return null;
        const step = detail.steps.find(value => value.step_id === record.step_id);
        const attempt = record.attempt_id ? detail.attempts.find(value => value.attempt_id === record.attempt_id && value.step_id === record.step_id)
          : latestAttemptForStep(detail.attempts, record.step_id);
        if (!step || (record.attempt_id && !attempt)) throw new Error('Delegation record target is unavailable');
        const participant = detail.participants.find(value => value.participant_id === (attempt?.participant_id ?? step.assigned_participant_id));
        return { executionId: record.execution_id, step, attempt, participant, participants: detail.participants,
          ...(record.attempt_id ? { attemptId: record.attempt_id, projectionKey: `${record.execution_id}:${record.step_id}:${record.attempt_id}` } : {}),
          refetch: execution.refetch };
      });
    } catch { if (active.current) setFailed(true); }
    finally { if (active.current) setOpeningId(undefined); }
  };
  const first = records[0];
  if (!first) return null;
  const batch = records.length > 1;
  const action = t(`messages.delegation.action.${first.kind}`);
  const summary = batch ? t('messages.delegation.records', { count: records.length }) : first.title;
  const recordLabel = (record: DelegationRecord) => t('messages.delegation.openRecord', { title: record.title, interpolation: { escapeValue: false } });
  return <div className='turn-process-trace-delegation' data-delegation-record-id={batch ? undefined : first.id} data-delegation-record-kind={first.kind}
    data-delegation-record-count={records.length}>
    <button type='button' className='turn-process-trace-delegation__row'
      onClick={() => batch ? setExpanded(value => !value) : void open(first)}
      disabled={!batch && (!execution || Boolean(openingId))} aria-busy={!batch && openingId === first.id}
      aria-expanded={batch ? expanded : undefined} aria-controls={batch ? detailsId : undefined}
      aria-label={batch ? t('messages.delegation.toggleRecords', { action, count: records.length }) : recordLabel(first)}>
      <span className='turn-process-trace-delegation__leading' aria-hidden='true'>
        <ShareOne theme='outline' size='14' strokeWidth={3} className='turn-process-trace-delegation__symbol' />
        <Right theme='outline' size='12' className='turn-process-trace-delegation__disclosure' />
      </span>
      <span className='turn-process-trace-delegation__action'>{action}</span>
      <span className='turn-process-trace-delegation__separator' aria-hidden='true' />
      <span className='turn-process-trace-delegation__summary' title={summary}>{summary}</span>
    </button>
    {batch && expanded && <div id={detailsId} className='turn-process-trace-delegation__records' role='list'>
      {records.map(record => <div key={record.id} role='listitem'>
        <button type='button' className='turn-process-trace-delegation__task' data-delegation-record-id={record.id} disabled={!execution || Boolean(openingId)}
          aria-label={recordLabel(record)} aria-busy={openingId === record.id} onClick={() => void open(record)}>
          {record.title}
        </button>
      </div>)}
    </div>}
    {failed && <span role='alert' className='turn-process-trace-detail__notice'>{t('messages.delegation.openFailed')}</span>}
  </div>;
}
