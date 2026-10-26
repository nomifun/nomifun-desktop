import type { OfficeItem, OfficeMutation } from '@/common/types/officeHub';
import { uuidv7 } from '@/common/utils/uuidv7';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { OfficeButton, OfficeDialog } from './dialogs';
import { documentTarget, identityKey, writeScopes } from './model';
import { WriteFeedback, type FormServices } from './NativeForm';
import { useOfficeWrite, type WriteIdentity } from './useOfficeWrite';

export interface DocumentDeleteSelection { item: OfficeItem; identity: WriteIdentity }
export function DocumentDeleteDialog({ selection, ...services }: FormServices & { selection: DocumentDeleteSelection }) {
  const { t } = useTranslation();
  const [clientId] = useState(uuidv7);
  const write = useOfficeWrite();
  const done = useRef<string | null>(null);
  const target = documentTarget(selection.item);
  const changed = selection.identity.key !== identityKey(services.status);
  useEffect(() => {
    if (write.result?.status === 'succeeded' && done.current !== write.result.id) {
      done.current = write.result.id; services.onSaved(write.result);
    }
  }, [write.result, services.onSaved]);
  const remove = async () => {
    if (!target || changed || selection.item.pending_operation_id) return;
    const mutation: OfficeMutation = { kind: 'document_delete', client_item_id: clientId, ...target, title: selection.item.title || target.document_id };
    const scopes = writeScopes(services.status, [mutation]);
    if (scopes.length) { services.authorize(scopes); return; }
    await write.save([mutation], selection.identity);
  };
  return <OfficeDialog title={t('office.documents.deleteTitle')} hidden={services.hidden} busy={write.loading} onClose={services.onClose} footer={<>
    <OfficeButton disabled={write.loading} onClick={services.onClose}>{t('office.actions.cancel')}</OfficeButton>
    <OfficeButton className='office-button--danger' disabled={!target || changed || !!selection.item.pending_operation_id || write.loading || write.locked} onClick={() => void remove()}>{t('office.actions.delete')}</OfficeButton>
  </>}>
    <strong className='office-break'>{selection.item.title || selection.item.id}</strong>
    {selection.item.url && <p className='office-small office-muted office-break'>{selection.item.url}</p>}
    <p>{t('office.documents.deleteHint')}</p>
    {changed && <p className='office-warning'>{t('office.operation.identityChangedInput')}</p>}
    <WriteFeedback write={write} onResult={services.onResult} errorPanel={services.errorPanel} />
  </OfficeDialog>;
}
