import type { OfficeItem, OfficeMutation, OfficeOperation, OfficeStatus } from '@/common/types/officeHub';
import { uuidv7 } from '@/common/utils/uuidv7';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { OfficeButton } from './dialogs';
import { identityKey, writeScopes } from './model';
import { captureIdentity, useOfficeWrite, type WriteIdentity } from './useOfficeWrite';

export function TaskRow({ item, status, compact, authorize, onResult, onResultId, onSaved, errorPanel }: {
  item: OfficeItem; status: OfficeStatus | null; compact?: boolean; authorize: (scopes: string[]) => void;
  onResult: (operation: OfficeOperation) => void; onResultId: (id: string) => void; onSaved: () => void;
  errorPanel: (error: unknown) => React.ReactNode;
}) {
  const { t } = useTranslation();
  const [intent, setIntent] = useState<{ identity: WriteIdentity; clientId: string; completed: boolean } | null>(null);
  const [error, setError] = useState<unknown>(null);
  const write = useOfficeWrite();
  const done = useRef<string | null>(null);
  useEffect(() => { if (write.result?.status === 'succeeded' && done.current !== write.result.id) { done.current = write.result.id; onSaved(); } }, [write.result, onSaved]);
  const toggle = async () => {
    const identity = captureIdentity(status);
    if (!identity || item.pending_operation_id) return;
    const next = intent ?? { identity, clientId: uuidv7(), completed: !item.completed };
    setIntent(next);
    if (next.identity.key !== identityKey(status)) { setError(new Error('office.operation.identityChangedInput')); return; }
    const mutation: OfficeMutation = { kind: 'task_complete', client_item_id: next.clientId, task_id: item.id, completed: next.completed };
    const scopes = writeScopes(status, [mutation]);
    if (scopes.length) { authorize(scopes); return; }
    const result = await write.save([mutation], next.identity);
    if (result && result.status !== 'succeeded') onResult(result);
  };
  const person = (id: string) => id === status?.actor?.user_id ? status.actor.name || id : id;
  return <div className='office-task'><label><input type='checkbox' checked={write.result?.status === 'succeeded' && intent ? intent.completed : !!item.completed} disabled={!!item.pending_operation_id || write.loading || write.locked || (!!intent && intent.identity.key !== identityKey(status))} onChange={() => void toggle()} /><span>{item.title || item.id}</span></label>{!compact && <p className='office-small office-muted'>{t('office.tasks.assignee')}: {item.assignee_ids.map(person).join(', ') || '—'} · {t('office.tasks.creator')}: {item.creator_id ? person(item.creator_id) : '—'} · {item.tasklist_ids.join(', ') || t('office.tasks.noList')}</p>}{item.due && <p className='office-small office-muted'>{item.due}</p>}{errorPanel(error)}{errorPanel(write.error)}
    {item.pending_operation_id ? <OfficeButton quiet onClick={() => onResultId(item.pending_operation_id!)}>{t('office.operation.status.' + (item.pending_status ?? 'unknown'))}</OfficeButton> : write.result && write.result.status !== 'succeeded' ? <OfficeButton quiet onClick={() => onResult(write.result!)}>{t('office.operation.status.' + write.result.status)}</OfficeButton> : write.locked && !write.result ? <OfficeButton quiet disabled={write.loading} onClick={() => void write.recover()}>{t('office.operation.checkReceipt')}</OfficeButton> : null}
  </div>;
}
