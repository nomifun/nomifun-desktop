import type { OfficeMutation, OfficeOperation, OfficeStatus } from '@/common/types/officeHub';
import { uuidv7 } from '@/common/utils/uuidv7';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { OfficeBadge, OfficeButton, OfficeDialog } from './dialogs';
import { identityKey, writeScopes } from './model';
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
export function NativeForm({ identity, ...services }: FormServices & { identity: WriteIdentity }) {
  const { t } = useTranslation();
  const [clientId] = useState(uuidv7);
  const [title, setTitle] = useState('');
  const [validation, setValidation] = useState<unknown>(null);
  const changed = identity.key !== identityKey(services.status);
  const write = useOfficeWrite();
  const done = useRef<string | null>(null);
  useEffect(() => { if (write.result?.status === 'succeeded' && done.current !== write.result.id) { done.current = write.result.id; services.onSaved(write.result); } }, [write.result, services.onSaved]);
  const save = async () => {
    setValidation(null);
    try {
      if (!title.trim() || changed) throw new Error('office.validation.required');
      const item: OfficeMutation = { kind: 'document_create', client_item_id: clientId, title: title.trim() };
      const scopes = writeScopes(services.status, [item]);
      if (scopes.length) { services.authorize(scopes); return; }
      await write.save([item], identity);
    } catch (error) { setValidation(error); }
  };
  return <OfficeDialog title={t('office.actions.createDocument')} hidden={services.hidden} onClose={services.onClose} busy={write.loading} footer={<><span className='office-muted office-small'>{t('office.operation.actor', { name: identity.actor.name || identity.actor.user_id })}</span><div className='office-actions'><OfficeButton disabled={write.loading} onClick={services.onClose}>{t('office.actions.cancel')}</OfficeButton><OfficeButton primary disabled={write.loading || write.locked || changed} onClick={() => void save()}>{t('office.actions.create')}</OfficeButton></div></>}>
    {changed && <p className='office-warning'>{t('office.operation.identityChangedInput')}</p>}{services.errorPanel(validation)}<WriteFeedback write={write} onResult={services.onResult} errorPanel={services.errorPanel} />
    <fieldset disabled={write.loading || write.locked || changed} className='office-fieldset'>
      <label className='office-label'>{t('office.documents.title')}<input className='office-input' autoFocus value={title} onChange={event => setTitle(event.target.value)} /></label>
      <p className='office-small office-muted'>{t('office.documents.createHint')}</p>
    </fieldset>
  </OfficeDialog>;
}
