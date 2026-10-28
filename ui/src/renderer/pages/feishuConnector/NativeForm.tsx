import type { FeishuMutation, FeishuOperation, FeishuStatus } from '@/common/types/feishuConnector';
import { uuidv7 } from '@/common/utils/uuidv7';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { FeishuBadge, FeishuButton, FeishuDialog } from './dialogs';
import { identityKey, writeScopes } from './model';
import { useFeishuWrite, type WriteIdentity } from './useFeishuWrite';

export interface FormServices {
  status: FeishuStatus | null; ownerKey: string; hidden: boolean;
  authorize: (scopes: string[]) => void; onClose: () => void;
  onSaved: (operation: FeishuOperation) => void; onResult: (operation: FeishuOperation) => void;
  errorPanel: (error: unknown) => React.ReactNode;
}
export function WriteFeedback({ write, onResult, errorPanel }: { write: ReturnType<typeof useFeishuWrite>; onResult: (operation: FeishuOperation) => void; errorPanel: FormServices['errorPanel'] }) {
  const { t } = useTranslation();
  const processing = write.result && ['queued', 'running'].includes(write.result.status);
  return <>{errorPanel(write.error)}{write.loading ? <p className='feishu-muted feishu-small' role='status'>{t('feishu.operation.saving')}</p> : processing ? <p className='feishu-muted feishu-small' role='status'>{t('feishu.operation.processing')}</p> : <>
    {write.locked && !write.result && <div className='feishu-warning'><p>{t('feishu.operation.networkHint')}</p><FeishuButton onClick={() => void write.recover()}>{t('feishu.operation.checkReceipt')}</FeishuButton></div>}
    {write.result && write.result.status !== 'succeeded' && <div className='feishu-inline-result' role='status'><FeishuBadge state={write.result.status === 'not_executed' ? 'neutral' : 'warning'}>{t('feishu.operation.status.' + write.result.status)}</FeishuBadge><span>{t(write.result.status === 'not_executed' ? 'feishu.operation.notExecutedHint' : 'feishu.operation.pendingHint')}</span><FeishuButton quiet onClick={() => onResult(write.result!)}>{t('feishu.actions.details')}</FeishuButton></div>}
  </>}</>;
}
export function NativeForm({ identity, ...services }: FormServices & { identity: WriteIdentity }) {
  const { t } = useTranslation();
  const [clientId] = useState(uuidv7);
  const [title, setTitle] = useState('');
  const [validation, setValidation] = useState<unknown>(null);
  const changed = identity.key !== identityKey(services.status);
  const write = useFeishuWrite();
  const done = useRef<string | null>(null);
  useEffect(() => { if (write.result?.status === 'succeeded' && done.current !== write.result.id) { done.current = write.result.id; services.onSaved(write.result); } }, [write.result, services.onSaved]);
  const save = async () => {
    setValidation(null);
    try {
      if (!title.trim() || changed) throw new Error('feishu.validation.required');
      const item: FeishuMutation = { kind: 'document_create', client_item_id: clientId, title: title.trim() };
      const scopes = writeScopes(services.status, [item]);
      if (scopes.length) { services.authorize(scopes); return; }
      await write.save([item], identity);
    } catch (error) { setValidation(error); }
  };
  return <FeishuDialog title={t('feishu.actions.createDocument')} hidden={services.hidden} onClose={services.onClose} busy={write.loading} footer={<><span className='feishu-muted feishu-small'>{t('feishu.operation.actor', { name: identity.actor.name || identity.actor.user_id })}</span><div className='feishu-actions'><FeishuButton disabled={write.loading} onClick={services.onClose}>{t('feishu.actions.cancel')}</FeishuButton><FeishuButton primary disabled={write.loading || write.locked || changed} onClick={() => void save()}>{t('feishu.actions.create')}</FeishuButton></div></>}>
    {changed && <p className='feishu-warning'>{t('feishu.operation.identityChangedInput')}</p>}{services.errorPanel(validation)}<WriteFeedback write={write} onResult={services.onResult} errorPanel={services.errorPanel} />
    <fieldset disabled={write.loading || write.locked || changed} className='feishu-fieldset'>
      <label className='feishu-label'>{t('feishu.documents.title')}<input className='feishu-input' autoFocus value={title} onChange={event => setTitle(event.target.value)} /></label>
      <p className='feishu-small feishu-muted'>{t('feishu.documents.createHint')}</p>
    </fieldset>
  </FeishuDialog>;
}
