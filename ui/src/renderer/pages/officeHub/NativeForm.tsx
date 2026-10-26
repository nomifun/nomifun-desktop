import type { OfficeItem, OfficeMutation, OfficeOperation, OfficeStatus } from '@/common/types/officeHub';
import { uuidv7 } from '@/common/utils/uuidv7';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { OfficeBadge, OfficeButton, OfficeDialog } from './dialogs';
import { hasPermission, identityKey, localDate, localEventTimes, localZone, scopesFor, writeScopes } from './model';
import { useOfficeQuery } from './useOfficeHub';
import { useOfficeWrite, type WriteIdentity } from './useOfficeWrite';

export interface FormServices {
  status: OfficeStatus | null; ownerKey: string; hidden: boolean;
  authorize: (scopes: string[]) => void; onClose: () => void;
  onSaved: (operation: OfficeOperation) => void; onResult: (operation: OfficeOperation) => void;
  errorPanel: (error: unknown) => React.ReactNode;
}
export function WriteFeedback({ write, onResult, errorPanel }: { write: ReturnType<typeof useOfficeWrite>; onResult: (operation: OfficeOperation) => void; errorPanel: FormServices['errorPanel'] }) {
  const { t } = useTranslation();
  const processing = write.result && ['queued', 'running'].includes(write.result.status);
  return <>{errorPanel(write.error)}{write.loading ? <p className='office-muted office-small' role='status'>{t('office.operation.saving')}</p> : processing ? <p className='office-muted office-small' role='status'>{t('office.operation.processing')}</p> : <>
    {write.locked && !write.result && <div className='office-warning'><p>{t('office.operation.networkHint')}</p><OfficeButton onClick={() => void write.recover()}>{t('office.operation.checkReceipt')}</OfficeButton></div>}
    {write.result && write.result.status !== 'succeeded' && <div className='office-inline-result' role='status'><OfficeBadge state={write.result.status === 'not_executed' ? 'neutral' : 'warning'}>{t('office.operation.status.' + write.result.status)}</OfficeBadge><span>{t(write.result.status === 'not_executed' ? 'office.operation.notExecutedHint' : 'office.operation.pendingHint')}</span><OfficeButton quiet onClick={() => onResult(write.result!)}>{t('office.actions.details')}</OfficeButton></div>}
  </>}</>;
}
export function NativeForm({ kind, identity, ...services }: FormServices & { kind: 'document' | 'task' | 'event'; identity: WriteIdentity }) {
  const { t } = useTranslation();
  const [clientId] = useState(uuidv7);
  const [title, setTitle] = useState('');
  const [assignee, setAssignee] = useState({ id: identity.actor.user_id, name: identity.actor.name });
  const [due, setDue] = useState('');
  const [tasklist, setTasklist] = useState('');
  const [peopleQuery, setPeopleQuery] = useState<string | null>(null);
  const [peopleInput, setPeopleInput] = useState('');
  const [peopleCache, setPeopleCache] = useState<OfficeItem[]>([]);
  const [listCache, setListCache] = useState<OfficeItem[]>([]);
  const [listCursor, setListCursor] = useState<string | null>(null);
  const [date, setDate] = useState(localDate);
  const [time, setTime] = useState('09:00');
  const [duration, setDuration] = useState(30);
  const [timezone] = useState(localZone);
  const [validation, setValidation] = useState<unknown>(null);
  const changed = identity.key !== identityKey(services.status);
  const lists = useOfficeQuery(kind === 'task' && !changed && hasPermission(services.status, 'tasklists.read') ? { kind: 'tasklists', cursor: listCursor } : null, services.ownerKey);
  const people = useOfficeQuery(kind === 'task' && !changed && peopleQuery !== null && hasPermission(services.status, 'people.read') ? { kind: 'people', query: peopleQuery } : null, services.ownerKey);
  const write = useOfficeWrite();
  const done = useRef<string | null>(null);
  useEffect(() => { if (lists.data) setListCache(previous => [...previous, ...lists.data!.items.filter(item => !previous.some(value => value.id === item.id))]); }, [lists.data]);
  useEffect(() => { if (people.data) setPeopleCache(previous => [...previous, ...people.data!.items.filter(item => !previous.some(value => value.id === item.id))]); }, [people.data]);
  useEffect(() => { if (write.result?.status === 'succeeded' && done.current !== write.result.id) { done.current = write.result.id; services.onSaved(write.result); } }, [write.result, services.onSaved]);
  const save = async () => {
    setValidation(null);
    try {
      if (!title.trim() || changed || (kind === 'task' && !assignee.id)) throw new Error('office.validation.required');
      let item: OfficeMutation;
      if (kind === 'document') item = { kind: 'document_create', client_item_id: clientId, title: title.trim() };
      else if (kind === 'task') item = { kind: 'task_create', client_item_id: clientId, title: title.trim(), assignee_id: assignee.id, assignee_name: assignee.name, due: due || null, tasklist_id: tasklist || null };
      else item = { kind: 'calendar_create', client_item_id: clientId, title: title.trim(), ...localEventTimes(date, time, duration), timezone };
      const scopes = writeScopes(services.status, [item]);
      if (scopes.length) { services.authorize(scopes); return; }
      await write.save([item], identity);
    } catch (error) { setValidation(error); }
  };
  const formTitle = t('office.actions.' + (kind === 'document' ? 'createDocument' : kind === 'task' ? 'createTask' : 'createEvent'));
  const options = [{ id: identity.actor.user_id, title: identity.actor.name || identity.actor.user_id }, ...peopleCache.filter(item => item.id !== identity.actor.user_id)];
  return <OfficeDialog title={formTitle} hidden={services.hidden} onClose={services.onClose} busy={write.loading} footer={<><span className='office-muted office-small'>{t('office.operation.actor', { name: identity.actor.name || identity.actor.user_id })}</span><div className='office-actions'><OfficeButton disabled={write.loading} onClick={services.onClose}>{t('office.actions.cancel')}</OfficeButton><OfficeButton primary disabled={write.loading || write.locked || changed} onClick={() => void save()}>{t('office.actions.create')}</OfficeButton></div></>}>
    {changed && <p className='office-warning'>{t('office.operation.identityChangedInput')}</p>}{services.errorPanel(validation)}<WriteFeedback write={write} onResult={services.onResult} errorPanel={services.errorPanel} />
    <fieldset disabled={write.loading || write.locked || changed} className='office-fieldset'>
      <label className='office-label'>{t(kind === 'task' ? 'office.tasks.title' : kind === 'event' ? 'office.calendar.title' : 'office.documents.title')}<input className='office-input' autoFocus value={title} onChange={event => setTitle(event.target.value)} /></label>
      {kind === 'document' && <p className='office-small office-muted'>{t('office.documents.createHint')}</p>}
      {kind === 'task' && <><div className='office-form-row'><label className='office-label'>{t('office.tasks.assignee')}<select className='office-input' value={assignee.id} onChange={event => setAssignee({ id: event.target.value, name: options.find(item => item.id === event.target.value)?.title ?? '' })}>{options.map(item => <option key={item.id} value={item.id}>{item.title}</option>)}</select></label><label className='office-label'>{t('office.tasks.due')}<input className='office-input' type='date' value={due} onChange={event => setDue(event.target.value)} /></label></div>
        <details><summary>{t('office.tasks.otherPerson')}</summary><div className='office-form-row'><input className='office-input' aria-label={t('office.tasks.personSearch')} value={peopleInput} onChange={event => setPeopleInput(event.target.value)} /><OfficeButton disabled={people.loading} onClick={() => hasPermission(services.status, 'people.read') ? setPeopleQuery(peopleInput.trim()) : services.authorize(scopesFor(services.status, ['people.read']))}>{t('office.actions.search')}</OfficeButton></div>{services.errorPanel(people.error)}</details>
        <label className='office-label'>{t('office.tasks.list')}<select className='office-input' value={tasklist} onChange={event => setTasklist(event.target.value)}><option value=''>{t('office.tasks.noList')}</option>{listCache.map(item => <option key={item.id} value={item.id}>{item.title}</option>)}</select></label>{!hasPermission(services.status, 'tasklists.read') && <OfficeButton quiet onClick={() => services.authorize(scopesFor(services.status, ['tasklists.read']))}>{t('office.tasks.readLists')}</OfficeButton>}{lists.data?.has_more && lists.data.next_cursor && <OfficeButton quiet onClick={() => setListCursor(lists.data!.next_cursor)}>{t('office.actions.loadMore')}</OfficeButton>}{services.errorPanel(lists.error)}<p className='office-small office-muted'>{t('office.tasks.listHint')}</p>
      </>}
      {kind === 'event' && <><div className='office-form-row'><label className='office-label'>{t('office.calendar.date')}<input className='office-input' type='date' value={date} onChange={event => setDate(event.target.value)} /></label><label className='office-label'>{t('office.calendar.time')}<input className='office-input' type='time' value={time} onChange={event => setTime(event.target.value)} /></label></div><label className='office-label'>{t('office.calendar.duration')}<select className='office-input' value={duration} onChange={event => setDuration(Number(event.target.value))}>{[15, 30, 60, 90, 120].map(minutes => <option key={minutes} value={minutes}>{minutes}</option>)}</select></label><p className='office-small office-muted'>{timezone} · {t('office.calendar.personal')}</p></>}
    </fieldset>
  </OfficeDialog>;
}
