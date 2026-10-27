/**
 * FeishuKnowledgePage — the connected Feishu knowledge space.
 *
 * Lists documents the connected account can search; content always opens in
 * the browser — NomiFun keeps browsing and account operations simple instead
 * of reproducing the Feishu editor. Connection management reuses the shared
 * connector dialog (MCP & 技能 → 连接器 owns the same configuration).
 */
import FeishuLogo from '@/renderer/assets/channel-logos/lark.svg';
import { officeHub } from '@/common/adapter/officeHubBridge';
import type { DocumentFilter, OfficeItem, OfficeOperation } from '@/common/types/officeHub';
import PageHeader from '@/renderer/components/layout/PageHeader';
import { Message } from '@arco-design/web-react';
import { Add, Down, FileText, Left, LinkOne, Refresh, Search, TableFile, User } from '@icon-park/react';
import { useCallback, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import { ConnectionDialog } from '@/renderer/pages/officeHub/ConnectionDialog';
import { ConnectionLanding } from '@/renderer/pages/officeHub/ConnectionLanding';
import { DocumentDeleteDialog, type DocumentDeleteSelection } from '@/renderer/pages/officeHub/DocumentDeleteDialog';
import { AuthorizationPanel, OfficeButton, OfficeDialog, OperationDialog } from '@/renderer/pages/officeHub/dialogs';
import { documentTarget, hasPermission, identityKey, officeResourceUrl, permissionKey, readGroups, scopesFor } from '@/renderer/pages/officeHub/model';
import { NativeForm } from '@/renderer/pages/officeHub/NativeForm';
import { useOfficeConnection } from '@/renderer/pages/officeHub/useOfficeConnection';
import { useOfficeQuery } from '@/renderer/pages/officeHub/useOfficeHub';
import { captureIdentity, type WriteIdentity } from '@/renderer/pages/officeHub/useOfficeWrite';
import '@/renderer/pages/officeHub/styles.css';

type Dialog = 'connection' | 'document' | 'document-delete' | 'result' | 'link' | null;

export default function FeishuKnowledgePage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const connection = useOfficeConnection({ onNeedSettings: () => setDialog('connection') });
  const { hub, busy, actionError, setActionError, authVisible, run, openExternal, startAuth, authorize, closeAuth, signedOut, errorPanel, authorization } = connection;
  const [dialog, setDialog] = useState<Dialog>(null);
  const [refreshKey, setRefreshKey] = useState(0);
  const [searchInput, setSearchInput] = useState('');
  const [searchTerm, setSearchTerm] = useState('');
  const [documentPages, setDocumentPages] = useState<{ key: string; cursors: Array<string | null> }>({ key: '', cursors: [null] });
  const [filter, setFilter] = useState<DocumentFilter>('all');
  const [deleteSelection, setDeleteSelection] = useState<DocumentDeleteSelection | null>(null);
  const [formIdentity, setFormIdentity] = useState<WriteIdentity | null>(null);
  const [link, setLink] = useState('');
  const selectedConnection = identityKey(hub.status);
  const queryOwnerKey = selectedConnection + ':' + permissionKey(hub.status) + ':' + refreshKey;
  const ready = hub.status?.runtime.phase === 'ready' && hub.status.connection.enabled && !!hub.status.actor && hub.status.phase === 'ready';
  const documentPageKey = JSON.stringify([selectedConnection, permissionKey(hub.status), searchTerm, filter]);
  const documentCursors = documentPages.key === documentPageKey ? documentPages.cursors : [null];
  const resetDocumentPages = () => setDocumentPages({ key: '', cursors: [null] });
  const documents = useOfficeQuery(ready && hasPermission(hub.status, 'documents.browse') ? { kind: 'documents', query: searchTerm, filter, cursor: documentCursors[documentCursors.length - 1] } : null, queryOwnerKey);
  const refresh = useCallback(() => { setRefreshKey(value => value + 1); }, []);
  const showOperation = (id: string) => run(async () => { await hub.openOperation(id); setDialog('result'); });
  const showResult = (operation: OfficeOperation) => { hub.setOperation(operation); setDialog('result'); refresh(); };
  const saved = (operation: OfficeOperation) => {
    hub.setOperation(operation); setDialog(null);
    const kind = operation.items[0]?.mutation.kind;
    Message.success({ content: t(kind === 'document_delete' ? 'office.deleted' : 'office.saved'), duration: 3000 });
    if (kind === 'document_create') {
      setSearchInput(''); setSearchTerm(''); resetDocumentPages(); setFilter('all');
    }
    refresh();
  };
  const errorView = (error: unknown) => errorPanel(error, (id) => void showOperation(id));
  const requestRead = (group: keyof typeof readGroups) => authorize(scopesFor(hub.status, readGroups[group]));
  const gate = (group: keyof typeof readGroups) => <div className='office-empty'><h3>{t('office.permissions.groups.' + group)}</h3><p>{t('office.permissions.readMissing')}</p><OfficeButton primary onClick={() => requestRead(group)}>{t('office.actions.authorizeRead')}</OfficeButton></div>;
  const newDocument = () => {
    const identity = captureIdentity(hub.status);
    if (!identity || !ready) { setDialog('connection'); return; }
    setFormIdentity(identity); setActionError(null); setDialog('document');
  };
  const openResource = async (item: Pick<OfficeItem, 'url'>) => {
    try {
      if (!item.url) throw new Error('office.resources.missingLink');
      await openExternal(officeResourceUrl(item.url));
    } catch (error) { setActionError(error); }
  };
  const deleteResource = (item: OfficeItem) => {
    const identity = captureIdentity(hub.status);
    if (!identity || !ready) { setDialog('connection'); return; }
    setDeleteSelection({ item, identity }); setActionError(null); setDialog('document-delete');
  };
  const nextDocumentCursor = documents.data?.next_cursor;
  const repeatedDocumentCursor = !!nextDocumentCursor && documentCursors.includes(nextDocumentCursor);
  const moveDocumentPage = (direction: 'previous' | 'next') => setDocumentPages(previous => {
    const cursors = previous.key === documentPageKey ? previous.cursors : [null];
    if (direction === 'previous') return { key: documentPageKey, cursors: cursors.length > 1 ? cursors.slice(0, -1) : cursors };
    if (!nextDocumentCursor || cursors.includes(nextDocumentCursor)) return previous;
    return { key: documentPageKey, cursors: [...cursors, nextDocumentCursor] };
  });
  const resourceTime = (item: OfficeItem) => {
    const value = item.modified_at;
    if (!value) return '—';
    const time = /^\d+$/.test(value) ? new Date(Number(value) * (value.length <= 10 ? 1000 : 1)) : new Date(value);
    return Number.isNaN(time.getTime()) ? '—' : time.toLocaleDateString();
  };
  const resources = documents.data?.items ?? [];
  const resourcesView = () => <section className='office-content'>
    <div className='office-filterbar'><div className='office-filters'>{(['all', 'document', 'base'] as const).map(value => <button type='button' key={value} className={filter === value ? 'active' : ''} onClick={() => { setFilter(value); resetDocumentPages(); }}>{t('office.resources.filters.' + value)}</button>)}</div><OfficeButton quiet aria-label={t('office.resources.openLink')} onClick={() => { setLink(''); setActionError(null); setDialog('link'); }}><LinkOne size={15} /></OfficeButton></div>
    <p className='office-small office-muted'>{t('office.resources.listHint')}</p>
      {documents.error ? <>{errorView(documents.error)}<OfficeButton onClick={documents.refresh}>{t('office.actions.check')}</OfficeButton></> : documents.loading || (!documents.data && hasPermission(hub.status, 'documents.browse')) ? <p className='office-empty' role='status'>{t('office.loading')}</p> : resources.length ? <div className='office-table-scroll'><table className='office-table office-resource-table'>
        <thead><tr><th>{t('office.resources.name')}</th><th>{t('office.resources.type')}</th><th>{t('office.resources.updated')}</th><th>{t('office.resources.actions')}</th></tr></thead>
        <tbody>{resources.map(item => <tr key={item.kind + item.id}>
          <td><button type='button' className='office-resource' disabled={!item.url} title={t(item.url ? 'office.resources.browserHint' : 'office.resources.missingLink')} onClick={() => void openResource(item)}>{item.kind === 'base' || item.kind === 'bitable' ? <TableFile size={19} /> : <FileText size={19} />}<strong>{item.title || item.id}</strong></button></td>
          <td className='office-muted office-small'>{t(item.kind === 'base' || item.kind === 'bitable' ? 'office.resources.filters.base' : 'office.resources.filters.document')}</td>
          <td className='office-muted office-small'>{resourceTime(item)}</td>
          <td><div className='office-actions'><OfficeButton quiet disabled={!item.url} onClick={() => void openResource(item)}>{t('office.actions.openBrowser')}</OfficeButton>
            {documentTarget(item) && <OfficeButton quiet disabled={!!item.pending_operation_id} aria-label={t('office.documents.deleteNamed', { title: item.title || item.id })} onClick={() => deleteResource(item)}>{t('office.actions.delete')}</OfficeButton>}
            {item.pending_operation_id && <OfficeButton quiet onClick={() => void showOperation(item.pending_operation_id!)}>{t('office.actions.original')}</OfficeButton>}
          </div></td>
        </tr>)}</tbody>
      </table></div> : !hasPermission(hub.status, 'documents.browse') ? gate('documents') : <div className='office-empty'><h3>{t('office.resources.queryEmpty')}</h3><OfficeButton onClick={() => { setLink(''); setActionError(null); setDialog('link'); }}>{t('office.resources.openLink')}</OfficeButton></div>}
      {documents.data?.warnings.map((warning, index) => <p className='office-warning office-small' key={index}>{warning}</p>)}
      {repeatedDocumentCursor && <p className='office-warning' role='alert'>{t('office.resources.paginationError')}</p>}
      {!documents.error && documents.data && (documents.data.has_more || documentCursors.length > 1) && <div className='office-pagination'><OfficeButton disabled={documentCursors.length === 1 || documents.loading} onClick={() => moveDocumentPage('previous')}>{t('office.actions.previous')}</OfficeButton><OfficeButton disabled={!documents.data.has_more || !nextDocumentCursor || repeatedDocumentCursor || documents.loading} onClick={() => moveDocumentPage('next')}>{t('office.actions.more')}</OfficeButton></div>}
  </section>;
  const formServices = { status: hub.status, ownerKey: queryOwnerKey, hidden: authVisible, authorize, onClose: () => setDialog(null), onSaved: saved, onResult: showResult, errorPanel: errorView };
  const content = !ready ? hub.status && <ConnectionLanding status={hub.status} busy={busy || hub.statusLoading} onPrepare={() => void run(async () => { try { await officeHub.prepare(); } finally { await hub.refreshStatus(); } })} onRepair={() => void run(async () => { try { await officeHub.repair(); } finally { await hub.refreshStatus(); } })} onConnect={() => {
    if (!hub.status!.connection.enabled && hub.status!.runtime.phase === 'ready') void run(async () => { await officeHub.updateConnection({ name: hub.status!.connection.name, profile: hub.status!.connection.profile, enabled: true }); await hub.refreshStatus(); setDialog('connection'); });
    else setDialog('connection');
  }} onSettings={() => setDialog('connection')} onCheck={() => void hub.refreshStatus()} /> : resourcesView();
  return <div className='office-hub'>
    <div className='feishu-back'>
      <button type='button' className='knowledge-detail-back-link inline-flex h-24px items-center gap-6px border-0 bg-transparent p-0 font-[inherit] text-12px leading-none text-[var(--color-text-3)] appearance-none cursor-pointer transition-colors hover:text-primary-6 focus-visible:outline-none focus-visible:text-primary-6' onClick={() => navigate('/knowledge')}>
        <span className='knowledge-detail-back-icon inline-flex h-14px w-14px items-center justify-center leading-none [&_svg]:block'><Left theme='outline' size='14' /></span>
        <span className='leading-none'>{t('knowledge.detail.back', { defaultValue: '返回知识库' })}</span>
      </button>
    </div>
    <PageHeader title={t('knowledge.feishu.title', { defaultValue: '飞书知识库' })} description={t('knowledge.feishu.description', { defaultValue: '浏览飞书云文档与多维表格，正文在浏览器中打开。' })} badge={<span className='feishu-badge'><img src={FeishuLogo} alt='' aria-hidden='true' />{t('office.feishu')}</span>} className='office-page-header page-header--flush' actions={<>{ready && <><form className='office-header-search' onSubmit={event => { event.preventDefault(); setSearchTerm(searchInput.trim()); resetDocumentPages(); }}><Search size={14} /><input aria-label={t('office.resources.search')} placeholder={t('office.resources.searchDocuments')} value={searchInput} onChange={event => setSearchInput(event.target.value)} /></form><OfficeButton primary onClick={newDocument}><Add size={14} />{t('office.actions.new')}</OfficeButton></>}<OfficeButton quiet disabled={hub.statusLoading || busy} aria-label={t('office.actions.refresh')} onClick={() => { void hub.refreshStatus(); refresh(); }}><Refresh size={15} /></OfficeButton><OfficeButton className='office-account-trigger' aria-label={t('office.connection.manage')} onClick={() => { setActionError(null); setDialog('connection'); }}><User size={16} /><span>{ready ? hub.status?.actor?.name || t('office.account.currentAccount') : t('office.connection.settings')}</span><Down size={12} /></OfficeButton></>} />
    <div className='office-body' inert={!!dialog || authVisible}>{errorView(hub.statusError)}{!dialog && !authVisible && errorView(actionError)}{hub.statusLoading && !hub.status ? <p className='office-empty'>{t('office.loading')}</p> : content}</div>
    {dialog === 'connection' && hub.status && <ConnectionDialog status={hub.status} authorization={authorization} onStart={startAuth} refresh={hub.refreshStatus} onClose={() => setDialog(null)} onError={setActionError} onSignedOut={() => { signedOut(); setFormIdentity(null); setDeleteSelection(null); refresh(); }} />}
    {dialog === 'document' && formIdentity && <NativeForm key={formIdentity.key} {...formServices} identity={formIdentity} />}
    {dialog === 'document-delete' && deleteSelection && <DocumentDeleteDialog {...formServices} selection={deleteSelection} onClose={() => { setDialog(null); refresh(); }} />}
    {dialog === 'result' && hub.operation && <OperationDialog operation={hub.operation} status={hub.status} busy={busy} error={actionError || hub.operationError ? <>{errorView(actionError)}{errorView(hub.operationError)}</> : null} onClose={() => { setDialog(null); refresh(); }} onReconcile={() => void run(async () => { hub.setOperation(await officeHub.reconcile(hub.operation!.id)); refresh(); })} onRefresh={() => void showOperation(hub.operation!.id)} onOpen={url => void openExternal(url)} />}
    {dialog === 'link' && <OfficeDialog title={t('office.resources.openLink')} onClose={() => setDialog(null)} footer={<><OfficeButton onClick={() => setDialog(null)}>{t('office.actions.cancel')}</OfficeButton><OfficeButton primary disabled={!link.trim()} onClick={() => {
      try { const url = officeResourceUrl(link); setDialog(null); void openResource({ url }); } catch (error) { setActionError(error); }
    }}>{t('office.actions.openBrowser')}</OfficeButton></>}><label className='office-label'>{t('office.resources.link')}<input className='office-input' value={link} onChange={event => setLink(event.target.value)} /></label><p className='office-small office-muted'>{t('office.resources.browserHint')}</p>{errorView(actionError)}</OfficeDialog>}
    {authVisible && <OfficeDialog title={t('office.auth.title')} onClose={closeAuth} busy={busy} footer={<OfficeButton primary disabled={busy} onClick={closeAuth}>{t('office.auth.return')}</OfficeButton>}><AuthorizationPanel {...authorization} /></OfficeDialog>}
  </div>;
}
