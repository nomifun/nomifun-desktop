import { ipcBridge } from '@/common';
import { officeHub } from '@/common/adapter/officeHubBridge';
import type { AuthStartRequest, DocumentFilter, OfficeItem, OfficeOperation } from '@/common/types/officeHub';
import PageHeader from '@/renderer/components/layout/PageHeader';
import { Message } from '@arco-design/web-react';
import { Add, Down, FileText, LinkOne, Refresh, Search, TableFile, User } from '@icon-park/react';
import { useCallback, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ConnectionDialog } from './ConnectionDialog';
import { ConnectionLanding } from './ConnectionLanding';
import { DocumentDeleteDialog, type DocumentDeleteSelection } from './DocumentDeleteDialog';
import { AuthorizationPanel, OfficeButton, OfficeDialog, OperationDialog } from './dialogs';
import { NativeForm } from './NativeForm';
import { OfficeAppSwitcher } from './OfficeAppSwitcher';
import { errorDetails, hasPermission, identityKey, officeError, documentTarget, officeResourceUrl, permissionKey, readGroups, relatedAuthorization, relatedOperation, scopesFor, terminalAuth } from './model';
import { useOfficeHub, useOfficeQuery } from './useOfficeHub';
import { captureIdentity, type WriteIdentity } from './useOfficeWrite';
import './styles.css';

type Dialog = 'connection' | 'document' | 'document-delete' | 'result' | 'link' | null;
export default function OfficeHubPage() {
  const { t } = useTranslation();
  const hub = useOfficeHub();
  const [dialog, setDialog] = useState<Dialog>(null);
  const [authVisible, setAuthVisible] = useState(false);
  const [busy, setBusy] = useState(false);
  const [actionError, setActionError] = useState<unknown>(null);
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
  const run = async (action: () => Promise<void>) => {
    if (busy) return;
    setBusy(true); setActionError(null);
    try { await action(); } catch (error) { setActionError(error); }
    finally { setBusy(false); }
  };
  const openExternal = async (url: string) => { try { if (new URL(url).protocol !== 'https:') throw new Error(t('office.error')); await ipcBridge.shell.openExternal.invoke(url); return true; } catch (error) { setActionError(error); return false; } };
  const startAuth = async (request: AuthStartRequest) => {
    if (hub.flow && !terminalAuth(hub.flow.phase)) return;
    await run(async () => {
      try { hub.setFlow(await officeHub.startAuth(request)); }
      catch (error) { const flow = relatedAuthorization(error); if (!flow) throw error; hub.setFlow(await officeHub.auth(flow)); }
      await hub.refreshStatus();
    });
  };
  const authorize = (scopes: string[]) => {
    if (!hub.status?.connection.profile || hub.status.runtime.phase !== 'ready') { setDialog('connection'); return; }
    setAuthVisible(true); setActionError(null);
    void startAuth({ kind: 'login', scopes: [...new Set(scopes)], profile: hub.status.connection.profile, app_id: null, app_secret: null });
  };
  const cancelAuth = () => run(async () => {
    if (!hub.flow) return;
    const next = await officeHub.cancelAuth(hub.flow.id); hub.setFlow(next);
    if (next.phase === 'complete' || next.phase === 'partial') await hub.refreshStatus();
  });
  const closeAuth = () => { if (hub.flow?.phase === 'complete' || hub.flow?.phase === 'partial') void hub.refreshStatus(); setAuthVisible(false); };
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
  const errorPanel = (error: unknown) => {
    if (!error) return null;
    const details = errorDetails(error), original = relatedOperation(error), flow = relatedAuthorization(error);
    const scopes = Array.isArray(details.missing_scopes) ? details.missing_scopes.filter((value): value is string => typeof value === 'string') : [];
    return <div className='office-error' role='alert'><span>{error instanceof Error && error.message.startsWith('office.') ? t(error.message) : officeError(error)}</span><div className='office-actions'>{original && <OfficeButton quiet onClick={() => void showOperation(original)}>{t('office.actions.original')}</OfficeButton>}{flow && <OfficeButton quiet onClick={() => void run(async () => { hub.setFlow(await officeHub.auth(flow)); setAuthVisible(true); })}>{t('office.auth.resume')}</OfficeButton>}{!!scopes.length && <OfficeButton quiet onClick={() => authorize(scopes)}>{t('office.actions.authorize')}</OfficeButton>}{typeof details.console_url === 'string' && <OfficeButton quiet onClick={() => void openExternal(details.console_url as string)}>{t('office.auth.admin')}</OfficeButton>}</div></div>;
  };
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
      {documents.error ? <>{errorPanel(documents.error)}<OfficeButton onClick={documents.refresh}>{t('office.actions.check')}</OfficeButton></> : documents.loading || (!documents.data && hasPermission(hub.status, 'documents.browse')) ? <p className='office-empty' role='status'>{t('office.loading')}</p> : resources.length ? <div className='office-table-scroll'><table className='office-table office-resource-table'>
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
  const formServices = { status: hub.status, ownerKey: queryOwnerKey, hidden: authVisible, authorize, onClose: () => setDialog(null), onSaved: saved, onResult: showResult, errorPanel };
  const authorization = { flow: hub.flow, busy, error: <>{errorPanel(actionError)}{errorPanel(hub.flowError)}</>, onCancel: () => void cancelAuth(), onOpen: (url: string) => void openExternal(url) };
  const content = !ready ? hub.status && <ConnectionLanding status={hub.status} busy={busy || hub.statusLoading} onPrepare={() => void run(async () => { try { await officeHub.prepare(); } finally { await hub.refreshStatus(); } })} onConnect={() => {
    if (!hub.status!.connection.enabled && hub.status!.runtime.phase === 'ready') void run(async () => { await officeHub.updateConnection({ name: hub.status!.connection.name, profile: hub.status!.connection.profile, enabled: true }); await hub.refreshStatus(); setDialog('connection'); });
    else setDialog('connection');
  }} onSettings={() => setDialog('connection')} onCheck={() => void hub.refreshStatus()} /> : resourcesView();
  return <div className='office-hub'>
    <PageHeader title={t('office.title')} badge={<OfficeAppSwitcher />} className='office-page-header page-header--flush' actions={<>{ready && <><form className='office-header-search' onSubmit={event => { event.preventDefault(); setSearchTerm(searchInput.trim()); resetDocumentPages(); }}><Search size={14} /><input aria-label={t('office.resources.search')} placeholder={t('office.resources.searchDocuments')} value={searchInput} onChange={event => setSearchInput(event.target.value)} /></form><OfficeButton primary onClick={newDocument}><Add size={14} />{t('office.actions.new')}</OfficeButton></>}<OfficeButton quiet disabled={hub.statusLoading || busy} aria-label={t('office.actions.refresh')} onClick={() => { void hub.refreshStatus(); refresh(); }}><Refresh size={15} /></OfficeButton><OfficeButton className='office-account-trigger' aria-label={t('office.connection.manage')} onClick={() => { setActionError(null); setDialog('connection'); }}><User size={16} /><span>{ready ? hub.status?.actor?.name || t('office.account.currentAccount') : t('office.connection.settings')}</span><Down size={12} /></OfficeButton></>} />
    <div className='office-body' inert={!!dialog || authVisible}>{errorPanel(hub.statusError)}{!dialog && !authVisible && errorPanel(actionError)}{hub.statusLoading && !hub.status ? <p className='office-empty'>{t('office.loading')}</p> : content}</div>
    {dialog === 'connection' && hub.status && <ConnectionDialog status={hub.status} authorization={authorization} onStart={startAuth} refresh={hub.refreshStatus} onClose={() => setDialog(null)} onError={setActionError} onSignedOut={() => { hub.setFlow(null); hub.setOperation(null); setFormIdentity(null); setDeleteSelection(null); setActionError(null); refresh(); }} />}
    {dialog === 'document' && formIdentity && <NativeForm key={formIdentity.key} {...formServices} identity={formIdentity} />}
    {dialog === 'document-delete' && deleteSelection && <DocumentDeleteDialog {...formServices} selection={deleteSelection} onClose={() => { setDialog(null); refresh(); }} />}
    {dialog === 'result' && hub.operation && <OperationDialog operation={hub.operation} status={hub.status} busy={busy} error={actionError || hub.operationError ? <>{errorPanel(actionError)}{errorPanel(hub.operationError)}</> : null} onClose={() => { setDialog(null); refresh(); }} onReconcile={() => void run(async () => { hub.setOperation(await officeHub.reconcile(hub.operation!.id)); refresh(); })} onRefresh={() => void showOperation(hub.operation!.id)} onOpen={url => void openExternal(url)} />}
    {dialog === 'link' && <OfficeDialog title={t('office.resources.openLink')} onClose={() => setDialog(null)} footer={<><OfficeButton onClick={() => setDialog(null)}>{t('office.actions.cancel')}</OfficeButton><OfficeButton primary disabled={!link.trim()} onClick={() => {
      try { const url = officeResourceUrl(link); setDialog(null); void openResource({ url }); } catch (error) { setActionError(error); }
    }}>{t('office.actions.openBrowser')}</OfficeButton></>}><label className='office-label'>{t('office.resources.link')}<input className='office-input' value={link} onChange={event => setLink(event.target.value)} /></label><p className='office-small office-muted'>{t('office.resources.browserHint')}</p>{errorPanel(actionError)}</OfficeDialog>}
    {authVisible && <OfficeDialog title={t('office.auth.title')} onClose={closeAuth} busy={busy} footer={<OfficeButton primary disabled={busy} onClick={closeAuth}>{t('office.auth.return')}</OfficeButton>}><AuthorizationPanel {...authorization} /></OfficeDialog>}
  </div>;
}
