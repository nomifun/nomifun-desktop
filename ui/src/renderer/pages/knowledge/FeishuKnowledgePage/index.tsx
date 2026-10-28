/**
 * FeishuKnowledgePage — the connected Feishu knowledge space.
 *
 * Lists documents the connected account can search; content always opens in
 * the browser — NomiFun keeps browsing and account operations simple instead
 * of reproducing the Feishu editor. Connection management reuses the shared
 * connector dialog (MCP & 技能 → 连接器 owns the same configuration).
 */
import FeishuLogo from '@/renderer/assets/channel-logos/lark.svg';
import { feishuConnector } from '@/common/adapter/feishuConnectorBridge';
import type { DocumentFilter, FeishuItem, FeishuOperation } from '@/common/types/feishuConnector';
import PageHeader from '@/renderer/components/layout/PageHeader';
import { Message } from '@arco-design/web-react';
import { Add, Down, FileText, Left, LinkOne, Refresh, Search, TableFile, User } from '@icon-park/react';
import { useCallback, useState } from 'react';
import { addKnowledgeContent, useKnowledgeBases } from '../useKnowledge';

/** Loads candidate Feishu bases only while the save dialog is mounted so the
 * workbench page itself never issues knowledge requests. */
function SaveToBaseDialog({ item, onClose, onError, onSaved }: {
  item: FeishuItem;
  onClose: () => void;
  onError: (error: unknown) => void;
  onSaved: () => void;
}) {
  const { t } = useTranslation();
  const { bases } = useKnowledgeBases();
  const [saving, setSaving] = useState(false);
  const feishuBases = bases.filter(base => base.kind === 'feishu');
  const confirm = async (knowledgeBaseId: Parameters<typeof addKnowledgeContent>[0]) => {
    if (!item.url || saving) return;
    setSaving(true);
    try {
      await addKnowledgeContent(knowledgeBaseId, { type: 'feishu', docs: [{ url: item.url, title: item.title || undefined }] });
      onSaved();
      Message.success({ content: t('knowledge.feishu.savedToBase', { defaultValue: '已加入知识库索引' }), duration: 3000 });
    } catch (error) { onError(error); }
    finally { setSaving(false); }
  };
  return (
    <FeishuDialog title={t('knowledge.feishu.saveToBase', { defaultValue: '存入知识库' })} onClose={() => { if (!saving) onClose(); }} footer={<FeishuButton onClick={onClose} disabled={saving}>{t('feishu.actions.cancel')}</FeishuButton>}>
      <p className='feishu-small feishu-muted'>{t('knowledge.feishu.saveToBaseHint', { defaultValue: '把「{{title}}」加入一个飞书知识库的名称索引；正文保持远端，不落本地。', title: item.title || item.id })}</p>
      {feishuBases.length === 0
        ? <div className='feishu-empty'><h3>{t('knowledge.feishu.noFeishuBase', { defaultValue: '还没有飞书知识库' })}</h3><p>{t('knowledge.feishu.noFeishuBaseHint', { defaultValue: '先在知识库列表新建一个「飞书文档」类型的知识库。' })}</p></div>
        : <div>{feishuBases.map(base => <button key={base.knowledge_base_id} type='button' className='feishu-resource' style={{ display: 'flex', width: '100%', padding: '8px 0' }} disabled={saving} onClick={() => void confirm(base.knowledge_base_id)}><FileText size={18} /><strong>{base.name}</strong></button>)}</div>}
    </FeishuDialog>
  );
}
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import { ConnectionDialog } from '@/renderer/pages/feishuConnector/ConnectionDialog';
import { ConnectionLanding } from '@/renderer/pages/feishuConnector/ConnectionLanding';
import { DocumentDeleteDialog, type DocumentDeleteSelection } from '@/renderer/pages/feishuConnector/DocumentDeleteDialog';
import { AuthorizationPanel, FeishuButton, FeishuDialog, OperationDialog } from '@/renderer/pages/feishuConnector/dialogs';
import { documentTarget, hasPermission, identityKey, feishuResourceUrl, permissionKey, readGroups, scopesFor } from '@/renderer/pages/feishuConnector/model';
import { NativeForm } from '@/renderer/pages/feishuConnector/NativeForm';
import { useFeishuConnection } from '@/renderer/pages/feishuConnector/useFeishuConnection';
import { useFeishuQuery } from '@/renderer/pages/feishuConnector/useFeishuConnector';
import { captureIdentity, type WriteIdentity } from '@/renderer/pages/feishuConnector/useFeishuWrite';
import '@/renderer/pages/feishuConnector/styles.css';

type Dialog = 'connection' | 'document' | 'document-delete' | 'result' | 'link' | 'save-to-base' | null;

export default function FeishuKnowledgePage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const connection = useFeishuConnection({ onNeedSettings: () => setDialog('connection') });
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
  const [saveTarget, setSaveTarget] = useState<FeishuItem | null>(null);
  const selectedConnection = identityKey(hub.status);
  const queryOwnerKey = selectedConnection + ':' + permissionKey(hub.status) + ':' + refreshKey;
  const ready = hub.status?.runtime.phase === 'ready' && hub.status.connection.enabled && !!hub.status.actor && hub.status.phase === 'ready';
  const documentPageKey = JSON.stringify([selectedConnection, permissionKey(hub.status), searchTerm, filter]);
  const documentCursors = documentPages.key === documentPageKey ? documentPages.cursors : [null];
  const resetDocumentPages = () => setDocumentPages({ key: '', cursors: [null] });
  const documents = useFeishuQuery(ready && hasPermission(hub.status, 'documents.browse') ? { kind: 'documents', query: searchTerm, filter, cursor: documentCursors[documentCursors.length - 1] } : null, queryOwnerKey);
  const refresh = useCallback(() => { setRefreshKey(value => value + 1); }, []);
  const showOperation = (id: string) => run(async () => { await hub.openOperation(id); setDialog('result'); });
  const showResult = (operation: FeishuOperation) => { hub.setOperation(operation); setDialog('result'); refresh(); };
  const saved = (operation: FeishuOperation) => {
    hub.setOperation(operation); setDialog(null);
    const kind = operation.items[0]?.mutation.kind;
    Message.success({ content: t(kind === 'document_delete' ? 'feishu.deleted' : 'feishu.saved'), duration: 3000 });
    if (kind === 'document_create') {
      setSearchInput(''); setSearchTerm(''); resetDocumentPages(); setFilter('all');
    }
    refresh();
  };
  const errorView = (error: unknown) => errorPanel(error, (id) => void showOperation(id));
  const requestRead = (group: keyof typeof readGroups) => authorize(scopesFor(hub.status, readGroups[group]));
  const gate = (group: keyof typeof readGroups) => <div className='feishu-empty'><h3>{t('feishu.permissions.groups.' + group)}</h3><p>{t('feishu.permissions.readMissing')}</p><FeishuButton primary onClick={() => requestRead(group)}>{t('feishu.actions.authorizeRead')}</FeishuButton></div>;
  const newDocument = () => {
    const identity = captureIdentity(hub.status);
    if (!identity || !ready) { setDialog('connection'); return; }
    setFormIdentity(identity); setActionError(null); setDialog('document');
  };
  const openResource = async (item: Pick<FeishuItem, 'url'>) => {
    try {
      if (!item.url) throw new Error('feishu.resources.missingLink');
      await openExternal(feishuResourceUrl(item.url));
    } catch (error) { setActionError(error); }
  };
  const saveToBase = (item: FeishuItem) => {
    if (!item.url) { setActionError(new Error('feishu.resources.missingLink')); return; }
    setSaveTarget(item); setActionError(null); setDialog('save-to-base');
  };
  const deleteResource = (item: FeishuItem) => {
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
  const resourceTime = (item: FeishuItem) => {
    const value = item.modified_at;
    if (!value) return '—';
    const time = /^\d+$/.test(value) ? new Date(Number(value) * (value.length <= 10 ? 1000 : 1)) : new Date(value);
    return Number.isNaN(time.getTime()) ? '—' : time.toLocaleDateString();
  };
  const resources = documents.data?.items ?? [];
  const resourcesView = () => <section className='feishu-content'>
    <div className='feishu-filterbar'><div className='feishu-filters'>{(['all', 'document', 'base'] as const).map(value => <button type='button' key={value} className={filter === value ? 'active' : ''} onClick={() => { setFilter(value); resetDocumentPages(); }}>{t('feishu.resources.filters.' + value)}</button>)}</div><FeishuButton quiet aria-label={t('feishu.resources.openLink')} onClick={() => { setLink(''); setActionError(null); setDialog('link'); }}><LinkOne size={15} /></FeishuButton></div>
    <p className='feishu-small feishu-muted'>{t('feishu.resources.listHint')}</p>
      {documents.error ? <>{errorView(documents.error)}<FeishuButton onClick={documents.refresh}>{t('feishu.actions.check')}</FeishuButton></> : documents.loading || (!documents.data && hasPermission(hub.status, 'documents.browse')) ? <p className='feishu-empty' role='status'>{t('feishu.loading')}</p> : resources.length ? <div className='feishu-table-scroll'><table className='feishu-table feishu-resource-table'>
        <thead><tr><th>{t('feishu.resources.name')}</th><th>{t('feishu.resources.type')}</th><th>{t('feishu.resources.updated')}</th><th>{t('feishu.resources.actions')}</th></tr></thead>
        <tbody>{resources.map(item => <tr key={item.kind + item.id}>
          <td><button type='button' className='feishu-resource' disabled={!item.url} title={t(item.url ? 'feishu.resources.browserHint' : 'feishu.resources.missingLink')} onClick={() => void openResource(item)}>{item.kind === 'base' || item.kind === 'bitable' ? <TableFile size={19} /> : <FileText size={19} />}<strong>{item.title || item.id}</strong></button></td>
          <td className='feishu-muted feishu-small'>{t(item.kind === 'base' || item.kind === 'bitable' ? 'feishu.resources.filters.base' : 'feishu.resources.filters.document')}</td>
          <td className='feishu-muted feishu-small'>{resourceTime(item)}</td>
          <td><div className='feishu-actions'><FeishuButton quiet disabled={!item.url} onClick={() => void openResource(item)}>{t('feishu.actions.openBrowser')}</FeishuButton>
            {(item.kind === 'doc' || item.kind === 'docx' || item.kind === 'wiki') &&
              <FeishuButton quiet disabled={!item.url} onClick={() => saveToBase(item)}>{t('knowledge.feishu.saveToBase', { defaultValue: '存入知识库' })}</FeishuButton>}
            {documentTarget(item) && <FeishuButton quiet disabled={!!item.pending_operation_id} aria-label={t('feishu.documents.deleteNamed', { title: item.title || item.id })} onClick={() => deleteResource(item)}>{t('feishu.actions.delete')}</FeishuButton>}
            {item.pending_operation_id && <FeishuButton quiet onClick={() => void showOperation(item.pending_operation_id!)}>{t('feishu.actions.original')}</FeishuButton>}
          </div></td>
        </tr>)}</tbody>
      </table></div> : !hasPermission(hub.status, 'documents.browse') ? gate('documents') : <div className='feishu-empty'><h3>{t('feishu.resources.queryEmpty')}</h3><FeishuButton onClick={() => { setLink(''); setActionError(null); setDialog('link'); }}>{t('feishu.resources.openLink')}</FeishuButton></div>}
      {documents.data?.warnings.map((warning, index) => <p className='feishu-warning feishu-small' key={index}>{warning}</p>)}
      {repeatedDocumentCursor && <p className='feishu-warning' role='alert'>{t('feishu.resources.paginationError')}</p>}
      {!documents.error && documents.data && (documents.data.has_more || documentCursors.length > 1) && <div className='feishu-pagination'><FeishuButton disabled={documentCursors.length === 1 || documents.loading} onClick={() => moveDocumentPage('previous')}>{t('feishu.actions.previous')}</FeishuButton><FeishuButton disabled={!documents.data.has_more || !nextDocumentCursor || repeatedDocumentCursor || documents.loading} onClick={() => moveDocumentPage('next')}>{t('feishu.actions.more')}</FeishuButton></div>}
  </section>;
  const formServices = { status: hub.status, ownerKey: queryOwnerKey, hidden: authVisible, authorize, onClose: () => setDialog(null), onSaved: saved, onResult: showResult, errorPanel: errorView };
  const content = !ready ? hub.status && <ConnectionLanding status={hub.status} busy={busy || hub.statusLoading} onPrepare={() => void run(async () => { try { await feishuConnector.prepare(); } finally { await hub.refreshStatus(); } })} onRepair={() => void run(async () => { try { await feishuConnector.repair(); } finally { await hub.refreshStatus(); } })} onConnect={() => {
    if (!hub.status!.connection.enabled && hub.status!.runtime.phase === 'ready') void run(async () => { await feishuConnector.updateConnection({ name: hub.status!.connection.name, profile: hub.status!.connection.profile, enabled: true }); await hub.refreshStatus(); setDialog('connection'); });
    else setDialog('connection');
  }} onSettings={() => setDialog('connection')} onCheck={() => void hub.refreshStatus()} /> : resourcesView();
  return <div className='feishu-connector'>
    <div className='feishu-back'>
      <button type='button' className='knowledge-detail-back-link inline-flex h-24px items-center gap-6px border-0 bg-transparent p-0 font-[inherit] text-12px leading-none text-[var(--color-text-3)] appearance-none cursor-pointer transition-colors hover:text-primary-6 focus-visible:outline-none focus-visible:text-primary-6' onClick={() => navigate('/knowledge')}>
        <span className='knowledge-detail-back-icon inline-flex h-14px w-14px items-center justify-center leading-none [&_svg]:block'><Left theme='outline' size='14' /></span>
        <span className='leading-none'>{t('knowledge.detail.back', { defaultValue: '返回知识库' })}</span>
      </button>
    </div>
    <PageHeader title={t('knowledge.feishu.title', { defaultValue: '飞书知识库' })} description={t('knowledge.feishu.description', { defaultValue: '浏览飞书云文档与多维表格，正文在浏览器中打开。' })} badge={<span className='feishu-badge'><img src={FeishuLogo} alt='' aria-hidden='true' />{t('feishu.feishu')}</span>} className='feishu-page-header page-header--flush' actions={<>{ready && <><form className='feishu-header-search' onSubmit={event => { event.preventDefault(); setSearchTerm(searchInput.trim()); resetDocumentPages(); }}><Search size={14} /><input aria-label={t('feishu.resources.search')} placeholder={t('feishu.resources.searchDocuments')} value={searchInput} onChange={event => setSearchInput(event.target.value)} /></form><FeishuButton primary onClick={newDocument}><Add size={14} />{t('feishu.actions.new')}</FeishuButton></>}<FeishuButton quiet disabled={hub.statusLoading || busy} aria-label={t('feishu.actions.refresh')} onClick={() => { void hub.refreshStatus(); refresh(); }}><Refresh size={15} /></FeishuButton><FeishuButton className='feishu-account-trigger' aria-label={t('feishu.connection.manage')} onClick={() => { setActionError(null); setDialog('connection'); }}><User size={16} /><span>{ready ? hub.status?.actor?.name || t('feishu.account.currentAccount') : t('feishu.connection.settings')}</span><Down size={12} /></FeishuButton></>} />
    <div className='feishu-body' inert={!!dialog || authVisible}>{errorView(hub.statusError)}{!dialog && !authVisible && errorView(actionError)}{hub.statusLoading && !hub.status ? <p className='feishu-empty'>{t('feishu.loading')}</p> : content}</div>
    {dialog === 'connection' && hub.status && <ConnectionDialog status={hub.status} authorization={authorization} onStart={startAuth} refresh={hub.refreshStatus} onClose={() => setDialog(null)} onError={setActionError} onSignedOut={() => { signedOut(); setFormIdentity(null); setDeleteSelection(null); refresh(); }} />}
    {dialog === 'document' && formIdentity && <NativeForm key={formIdentity.key} {...formServices} identity={formIdentity} />}
    {dialog === 'document-delete' && deleteSelection && <DocumentDeleteDialog {...formServices} selection={deleteSelection} onClose={() => { setDialog(null); refresh(); }} />}
    {dialog === 'result' && hub.operation && <OperationDialog operation={hub.operation} status={hub.status} busy={busy} error={actionError || hub.operationError ? <>{errorView(actionError)}{errorView(hub.operationError)}</> : null} onClose={() => { setDialog(null); refresh(); }} onReconcile={() => void run(async () => { hub.setOperation(await feishuConnector.reconcile(hub.operation!.id)); refresh(); })} onRefresh={() => void showOperation(hub.operation!.id)} onOpen={url => void openExternal(url)} />}
    {dialog === 'save-to-base' && saveTarget && <SaveToBaseDialog item={saveTarget} onClose={() => setDialog(null)} onError={setActionError} onSaved={() => { setDialog(null); setSaveTarget(null); refresh(); }} />}
    {dialog === 'link' && <FeishuDialog title={t('feishu.resources.openLink')} onClose={() => setDialog(null)} footer={<><FeishuButton onClick={() => setDialog(null)}>{t('feishu.actions.cancel')}</FeishuButton><FeishuButton primary disabled={!link.trim()} onClick={() => {
      try { const url = feishuResourceUrl(link); setDialog(null); void openResource({ url }); } catch (error) { setActionError(error); }
    }}>{t('feishu.actions.openBrowser')}</FeishuButton></>}><label className='feishu-label'>{t('feishu.resources.link')}<input className='feishu-input' value={link} onChange={event => setLink(event.target.value)} /></label><p className='feishu-small feishu-muted'>{t('feishu.resources.browserHint')}</p>{errorView(actionError)}</FeishuDialog>}
    {authVisible && <FeishuDialog title={t('feishu.auth.title')} onClose={closeAuth} busy={busy} footer={<FeishuButton primary disabled={busy} onClick={closeAuth}>{t('feishu.auth.return')}</FeishuButton>}><AuthorizationPanel {...authorization} /></FeishuDialog>}
  </div>;
}
