import type { FeishuItem, FeishuMutation } from '@/common/types/feishuConnector';
import { uuidv7 } from '@/common/utils/uuidv7';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { FeishuButton, FeishuDialog } from './dialogs';
import { documentTarget, identityKey, writeScopes } from './model';
import { WriteFeedback, type FormServices } from './NativeForm';
import { useFeishuWrite, type WriteIdentity } from './useFeishuWrite';

export interface DocumentDeleteSelection { item: FeishuItem; identity: WriteIdentity }
export function DocumentDeleteDialog({ selection, ...services }: FormServices & { selection: DocumentDeleteSelection }) {
  const { t } = useTranslation();
  const [clientId] = useState(uuidv7);
  const write = useFeishuWrite();
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
    const mutation: FeishuMutation = { kind: 'document_delete', client_item_id: clientId, ...target, title: selection.item.title || target.document_id };
    const scopes = writeScopes(services.status, [mutation]);
    if (scopes.length) { services.authorize(scopes); return; }
    await write.save([mutation], selection.identity);
  };
  return <FeishuDialog title={t('feishu.documents.deleteTitle')} hidden={services.hidden} busy={write.loading} onClose={services.onClose} footer={<>
    <FeishuButton disabled={write.loading} onClick={services.onClose}>{t('feishu.actions.cancel')}</FeishuButton>
    <FeishuButton className='feishu-button--danger' disabled={!target || changed || !!selection.item.pending_operation_id || write.loading || write.locked} onClick={() => void remove()}>{t('feishu.actions.delete')}</FeishuButton>
  </>}>
    <strong className='feishu-break'>{selection.item.title || selection.item.id}</strong>
    {selection.item.url && <p className='feishu-small feishu-muted feishu-break'>{selection.item.url}</p>}
    <p>{t('feishu.documents.deleteHint')}</p>
    {changed && <p className='feishu-warning'>{t('feishu.operation.identityChangedInput')}</p>}
    <WriteFeedback write={write} onResult={services.onResult} errorPanel={services.errorPanel} />
  </FeishuDialog>;
}
