import type { OfficeDocument, OfficeDocumentBlock, OfficeMutation } from '@/common/types/officeHub';
import { uuidv7 } from '@/common/utils/uuidv7';
import MarkdownView from '@/renderer/components/Markdown';
import { ArrowLeft } from '@icon-park/react';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { OfficeButton } from './dialogs';
import { hasPermission, identityKey, writeScopes } from './model';
import { WriteFeedback, type FormServices } from './NativeForm';
import { useOfficeQuery } from './useOfficeHub';
import { captureIdentity, useOfficeWrite, type WriteIdentity } from './useOfficeWrite';

interface DocumentServices extends Omit<FormServices, 'onClose' | 'onSaved'> {
  documentId: string; onBack: () => void; onOpen: (url: string) => void; refreshKey: number; onResultId: (id: string) => void;
}
function DocumentBody({ content }: { content: string }) {
  return <MarkdownView compact hiddenCodeCopyButton allowHtml={false} allowUnverifiedImages={false}>{content}</MarkdownView>;
}
export function DocumentView(services: DocumentServices) {
  const { t } = useTranslation();
  const [editing, setEditing] = useState<WriteIdentity | null>(null);
  const [saved, setSaved] = useState(false);
  const read = useOfficeQuery(hasPermission(services.status, 'documents.read') ? { kind: 'document', document: services.documentId, editing: false } : null, services.ownerKey + ':' + services.refreshKey);
  const document = read.data?.document;
  if (editing) return <DocumentEdit {...services} identity={editing} onDone={confirmed => { setEditing(null); setSaved(!!confirmed); read.refresh(); }} />;
  return <><div className='office-filebar'><div className='office-file-name'><OfficeButton quiet aria-label={t('office.actions.back')} onClick={services.onBack}><ArrowLeft size={16} /></OfficeButton><strong>{document?.title || t('office.tabs.documents')}</strong>{document?.pending_status && <span className='office-small office-warning'>{t('office.operation.status.' + document.pending_status)}</span>}</div><div className='office-actions'>
    {document?.pending_operation_id ? <OfficeButton onClick={() => services.onResultId?.(document.pending_operation_id!)}>{t('office.actions.reconcile')}</OfficeButton> : <OfficeButton disabled={!document || !captureIdentity(services.status)} onClick={() => setEditing(captureIdentity(services.status))}>{t('office.actions.edit')}</OfficeButton>}
    {document?.url && <OfficeButton quiet onClick={() => services.onOpen(document.url!)}>{t('office.actions.open')}</OfficeButton>}
  </div></div>{saved && <p className='office-notice' role='status'>{t('office.saved')}</p>}{services.errorPanel(read.error)}{read.loading ? <p className='office-empty'>{t('office.loading')}</p> : document && <article className='office-document'><h2 className='office-document-title'>{document.title}</h2><DocumentBody content={document.content} /></article>}</>;
}
function DocumentEdit(services: DocumentServices & { identity: WriteIdentity; onDone: (confirmed?: boolean) => void }) {
  const { t } = useTranslation();
  const changed = identityKey(services.status) !== services.identity.key;
  const query = useOfficeQuery(!changed ? { kind: 'document', document: services.documentId, editing: true } : null, services.ownerKey);
  const [cached, setCached] = useState<OfficeDocument | null>(null);
  const [selection, setSelection] = useState<{ block: OfficeDocumentBlock; revision: string } | null>(null);
  const [input, setInput] = useState('');
  const [clientId] = useState(uuidv7);
  const [message, setMessage] = useState('');
  const document = query.data?.document ?? cached;
  const baseline = document?.edit_baseline;
  const write = useOfficeWrite();
  const done = useRef<string | null>(null);
  const uncertain = !write.loading && (write.result ? write.result.status === 'unknown' : write.locked);
  const progress = write.loading ? t('office.operation.saving') : write.result && ['queued', 'running'].includes(write.result.status) ? t('office.operation.processing') : write.locked ? t('office.operation.status.' + (write.result?.status ?? 'unknown')) : selection && input !== selection.block.content ? t('office.documents.unsaved') : t('office.documents.editing');
  useEffect(() => { if (query.data?.document) setCached(query.data.document); }, [query.data]);
  useEffect(() => { if (write.result?.status === 'succeeded' && done.current !== write.result.id) { done.current = write.result.id; services.onDone(true); } }, [write.result, services.onDone]);
  useEffect(() => { if (write.result && ['unknown', 'accepted_pending_confirmation'].includes(write.result.status)) void write.recover(); }, [services.refreshKey]);
  const select = (block: OfficeDocumentBlock) => {
    if (write.locked || document?.pending_operation_id || changed || !block.editable || block.kind !== 'paragraph' || !baseline) return;
    if (!block.id && !(baseline.can_insert_first && !baseline.blocks.length)) return;
    if (selection && input !== selection.block.content) { setMessage(t('office.documents.finishParagraph')); return; }
    setSelection({ block, revision: baseline.revision }); setInput(block.content); setMessage('');
  };
  const save = async () => {
    if (!selection || !document || changed || write.locked || input === selection.block.content) return;
    if (!input.trim()) { setMessage(t('office.documents.emptyParagraph')); return; }
    const item: OfficeMutation = { kind: 'document_save', client_item_id: clientId, document_id: document.id, revision: selection.revision, block_id: selection.block.id, content: input };
    const scopes = writeScopes(services.status, [item]);
    if (scopes.length) { services.authorize(scopes); return; }
    await write.save([item], services.identity);
  };
  return <><div className='office-filebar'><div className='office-file-name'><OfficeButton quiet aria-label={t('office.actions.back')} disabled={write.loading} onClick={() => services.onDone()}><ArrowLeft size={16} /></OfficeButton><strong>{document?.title || t('office.tabs.documents')}</strong><span className={'office-small ' + (uncertain ? 'office-warning' : 'office-muted')} role='status'>{progress}</span></div><div className='office-actions'><OfficeButton quiet disabled={write.loading} onClick={() => services.onDone()}>{t('office.actions.cancel')}</OfficeButton><OfficeButton primary disabled={!selection || input === selection.block.content || write.loading || write.locked || changed || !!document?.pending_operation_id} onClick={() => void save()}>{t('office.actions.save')}</OfficeButton>{document?.url && <OfficeButton quiet onClick={() => services.onOpen(document.url!)}>{t('office.actions.open')}</OfficeButton>}</div></div>
    {changed && <p className='office-warning'>{t('office.operation.identityChangedInput')}</p>}{services.errorPanel(query.error)}<WriteFeedback write={write} onResult={services.onResult} errorPanel={services.errorPanel} />
    {document?.pending_operation_id && <p className='office-warning'>{t('office.operation.pendingHint')}<OfficeButton quiet onClick={() => services.onResultId?.(document.pending_operation_id!)}>{t('office.actions.reconcile')}</OfficeButton></p>}
    {document ? <article className='office-document'><h2 className='office-document-title'>{document.title}</h2>{baseline ? <>{baseline.blocks.map((block, index) => <section className={'office-doc-block' + (selection?.block === block || (selection?.block.id && selection.block.id === block.id) ? ' active' : '')} key={block.id ?? index}>
      {!!block.id && selection?.block.id === block.id && block.kind === 'paragraph' ? <textarea className='office-paragraph-editor' aria-label={t('office.documents.paragraph')} value={input} rows={2} disabled={write.loading || write.locked || changed || !!document.pending_operation_id} onChange={event => { setInput(event.target.value); event.target.style.height = 'auto'; event.target.style.height = Math.max(64, Math.min(300, event.target.scrollHeight)) + 'px'; }} /> : !!block.id && block.editable && block.kind === 'paragraph' ? <button type='button' className='office-paragraph-button' disabled={write.locked || changed || !!document.pending_operation_id} onClick={() => select(block)} aria-label={t('office.documents.editParagraph', { text: block.content.slice(0,30) })}><DocumentBody content={block.content} /></button> : <DocumentBody content={block.content} />}
    </section>)}{baseline.can_insert_first && !baseline.blocks.length && (!selection ? <OfficeButton quiet disabled={changed || write.locked} onClick={() => select({ id: null, kind: 'paragraph', content: '', editable: true })}>{t('office.documents.firstParagraph')}</OfficeButton> : <textarea className='office-paragraph-editor' aria-label={t('office.documents.paragraph')} value={input} rows={2} disabled={write.loading || write.locked || changed} onChange={event => setInput(event.target.value)} />)}</> : <><DocumentBody content={document.content} /><p className='office-muted office-small'>{t('office.documents.uneditable')}</p></>}{message && <p role='status' className='office-warning'>{message}</p>}</article> : <p className='office-empty'>{t('office.loading')}</p>}
  </>;
}
