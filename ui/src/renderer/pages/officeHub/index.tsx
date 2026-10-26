import { ipcBridge } from '@/common';
import { officeHub } from '@/common/adapter/officeHubBridge';
import type { AuthStartRequest, OfficeItem, OfficeOperation, TaskRelation } from '@/common/types/officeHub';
import PageHeader from '@/renderer/components/layout/PageHeader';
import { Message } from '@arco-design/web-react';
import { Add, FileText, LinkOne, Refresh, Search, SettingConfig, TableFile } from '@icon-park/react';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useSearchParams } from 'react-router-dom';
import { BaseEditForm, BaseView, type BaseEditSelection } from './BaseView';
import { ConnectionDialog } from './ConnectionDialog';
import { DocumentView } from './DocumentView';
import { AuthorizationPanel, OfficeBadge, OfficeButton, OfficeDialog, OperationDialog } from './dialogs';
import { NativeForm } from './NativeForm';
import { errorDetails, hasPermission, identityKey, localDate, localRange, officeError, officeTab, OFFICE_TABS, operationKind, parseBaseLink, permissionKey, readGroups, relatedAuthorization, relatedOperation, scopesFor, terminalAuth } from './model';
import { useOfficeHub, useOfficeQuery } from './useOfficeHub';
import { captureIdentity, type WriteIdentity } from './useOfficeWrite';
import { TaskRow } from './TaskRow';
import './styles.css';

type Dialog = 'connection' | 'document' | 'task' | 'event' | 'base-edit' | 'result' | 'link' | null;
export default function OfficeHubPage() {
  const { t } = useTranslation();
  const [params, setParams] = useSearchParams();
  const tab = officeTab(params.get('tab'));
  const documentId = params.get('document');
  const hub = useOfficeHub();
  const [dialog, setDialog] = useState<Dialog>(null);
  const [authVisible, setAuthVisible] = useState(false);
  const [busy, setBusy] = useState(false);
  const [actionError, setActionError] = useState<unknown>(null);
  const [refreshKey, setRefreshKey] = useState(0);
  const [searchInput, setSearchInput] = useState('');
  const [searchTerm, setSearchTerm] = useState('');
  const [searchCursors, setSearchCursors] = useState<Array<string | null>>([null]);
  const [filter, setFilter] = useState<'all' | 'document' | 'base'>('all');
  const [date, setDate] = useState(localDate);
  const [relation, setRelation] = useState<TaskRelation>('assigned');
  const [tasklistId, setTasklistId] = useState('');
  const [taskCursors, setTaskCursors] = useState<Array<string | null>>([null]);
  const [listCursor, setListCursor] = useState<string | null>(null);
  const [listCache, setListCache] = useState<OfficeItem[]>([]);
  const [baseLocation, setBaseLocation] = useState<{ base_token: string; table_id: string } | null>(null);
  const [baseEdit, setBaseEdit] = useState<BaseEditSelection | null>(null);
  const [formIdentity, setFormIdentity] = useState<WriteIdentity | null>(null);
  const [link, setLink] = useState('');
  const selectedConnection = identityKey(hub.status);
  const queryOwnerKey = selectedConnection + ':' + permissionKey(hub.status) + ':' + refreshKey;
  const ready = hub.status?.runtime.phase === 'ready' && hub.status.connection.enabled && !!hub.status.actor && hub.status.phase === 'ready';
  const listing = !documentId && (tab === 'overview' || tab === 'documents');
  const recent = useOfficeQuery(ready && listing ? { kind: 'recent' } : null, queryOwnerKey);
  const search = useOfficeQuery(ready && tab === 'documents' && !!searchTerm && hasPermission(hub.status, 'documents.search') ? { kind: 'search', query: searchTerm, cursor: searchCursors[searchCursors.length - 1] } : null, queryOwnerKey);
  const range = useMemo(() => { try { return localRange(date); } catch { return null; } }, [date]);
  const agenda = useOfficeQuery(ready && (tab === 'overview' || tab === 'agenda') && hasPermission(hub.status, 'calendar.read') && range ? { kind: 'agenda', ...range } : null, queryOwnerKey);
  const tasks = useOfficeQuery(ready && (tab === 'overview' || tab === 'agenda') && hasPermission(hub.status, 'tasks.read') && (tab === 'overview' || relation !== 'tasklist' || tasklistId) ? { kind: 'tasks', relation: tab === 'overview' ? 'assigned' : relation, tasklist_id: relation === 'tasklist' && tab === 'agenda' ? tasklistId : null, cursor: taskCursors[taskCursors.length - 1] } : null, queryOwnerKey);
  const lists = useOfficeQuery(ready && tab === 'agenda' && relation === 'tasklist' && hasPermission(hub.status, 'tasklists.read') ? { kind: 'tasklists', cursor: listCursor } : null, queryOwnerKey);
  useEffect(() => { setTasklistId(''); setListCache([]); setListCursor(null); setTaskCursors([null]); setSearchCursors([null]); setBaseLocation(null); }, [selectedConnection]);
  useEffect(() => { if (lists.data) setListCache(previous => [...previous, ...lists.data!.items.filter(item => !previous.some(value => value.id === item.id))]); }, [lists.data]);
  useEffect(() => { if (tab === 'activity') void hub.refreshHistory(); }, [tab, ready, selectedConnection, hub.refreshHistory]);
  const refresh = useCallback(() => { setRefreshKey(value => value + 1); void hub.refreshHistory(); }, [hub.refreshHistory]);
  const run = async (action: () => Promise<void>) => {
    if (busy) return;
    setBusy(true); setActionError(null);
    try { await action(); } catch (error) { setActionError(error); }
    finally { setBusy(false); }
  };
  const navigate = (next: string, document?: string) => {
    setParams(previous => { const value = new URLSearchParams(previous); value.set('tab', next); if (document) value.set('document', document); else value.delete('document'); return value; });
    setActionError(null);
  };
  const openExternal = async (url: string) => { try { if (new URL(url).protocol !== 'https:') throw new Error(t('office.error')); await ipcBridge.shell.openExternal.invoke(url); } catch (error) { setActionError(error); } };
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
  const saved = (operation: OfficeOperation) => { hub.setOperation(operation); setDialog(null); Message.success({ content: t('office.saved'), duration: 3000 }); const created = operation.items.length === 1 && operation.items[0].mutation.kind === 'document_create' ? operation.items[0] : null; if (created && (created.remote_url || created.remote_id)) navigate('documents', created.remote_url || created.remote_id!); refresh(); };
  const errorPanel = (error: unknown) => {
    if (!error) return null;
    const details = errorDetails(error), original = relatedOperation(error), flow = relatedAuthorization(error);
    const scopes = Array.isArray(details.missing_scopes) ? details.missing_scopes.filter((value): value is string => typeof value === 'string') : [];
    return <div className='office-error' role='alert'><span>{error instanceof Error && error.message.startsWith('office.') ? t(error.message) : officeError(error)}</span><div className='office-actions'>{original && <OfficeButton quiet onClick={() => void showOperation(original)}>{t('office.actions.original')}</OfficeButton>}{flow && <OfficeButton quiet onClick={() => void run(async () => { hub.setFlow(await officeHub.auth(flow)); setAuthVisible(true); })}>{t('office.auth.resume')}</OfficeButton>}{!!scopes.length && <OfficeButton quiet onClick={() => authorize(scopes)}>{t('office.actions.authorize')}</OfficeButton>}{typeof details.console_url === 'string' && <OfficeButton quiet onClick={() => void openExternal(details.console_url as string)}>{t('office.auth.admin')}</OfficeButton>}</div></div>;
  };
  const requestRead = (group: keyof typeof readGroups) => authorize(scopesFor(hub.status, readGroups[group]));
  const gate = (group: keyof typeof readGroups) => <div className='office-empty'><h3>{t('office.permissions.groups.' + group)}</h3><p>{t('office.permissions.readMissing')}</p><OfficeButton primary onClick={() => requestRead(group)}>{t('office.actions.authorizeRead')}</OfficeButton></div>;
  const newForm = (kind: 'document' | 'task' | 'event') => {
    const identity = captureIdentity(hub.status);
    if (!identity || !ready) { setDialog('connection'); return; }
    setFormIdentity(identity); setActionError(null); setDialog(kind);
  };
  const openResource = (item: OfficeItem) => {
    if (item.kind === 'base' || item.kind === 'bitable') {
      if (item.base_token && item.table_id) { setBaseLocation({ base_token: item.base_token, table_id: item.table_id }); navigate('base'); }
      else if (item.url) { try { setBaseLocation(parseBaseLink(item.url)); navigate('base'); } catch (error) { setActionError(error); } }
    } else navigate('documents', item.url || item.id);
  };
  const taskRows = (items: OfficeItem[], compact = false) => items.map(item => <TaskRow key={item.id + ':' + String(item.completed)} item={item} status={hub.status} compact={compact} authorize={authorize} onResult={showResult} onResultId={id => void showOperation(id)} onSaved={() => { Message.success({ content: t('office.saved'), duration: 3000 }); refresh(); }} errorPanel={errorPanel} />);
  const resourceSource = tab === 'documents' && searchTerm ? search : recent;
  const searchingDocuments = tab === 'documents' && !!searchTerm;
  const resourceTime = (item: OfficeItem) => {
    const value = searchingDocuments ? item.modified_at : item.opened_at;
    if (!value) return '—';
    const time = /^\d+$/.test(value) ? new Date(Number(value) * (value.length <= 10 ? 1000 : 1)) : new Date(value);
    return Number.isNaN(time.getTime()) ? '—' : time.toLocaleDateString();
  };
  const resources = (resourceSource.data?.items ?? []).filter(item => {
    const isBase = item.kind === 'base' || item.kind === 'bitable';
    return (filter === 'all' || (filter === 'base' ? isBase : !isBase)) && (tab !== 'overview' || !searchTerm || item.title.toLowerCase().includes(searchTerm.toLowerCase()));
  });
  const resourcesView = () => <section className='office-content'><div className='office-filterbar'><div className='office-filters'>{(['all', 'document', 'base'] as const).map(value => <button type='button' key={value} className={filter === value ? 'active' : ''} onClick={() => setFilter(value)}>{t('office.resources.filters.' + value)}</button>)}</div><OfficeButton quiet aria-label={t('office.resources.openLink')} onClick={() => { setLink(''); setDialog('link'); }}><LinkOne size={15} /></OfficeButton></div><div className={tab === 'overview' && hasPermission(hub.status, 'calendar.read') ? 'office-work-grid' : ''}><div>{errorPanel(resourceSource.error)}{resourceSource.loading ? <p className='office-empty'>{t('office.loading')}</p> : resources.length ? <div className='office-table-scroll'><table className='office-table office-resource-table'><thead><tr><th>{t('office.resources.name')}</th><th>{t('office.resources.type')}</th><th>{t(searchingDocuments ? 'office.resources.updated' : 'office.resources.opened')}</th></tr></thead><tbody>{resources.map(item => <tr key={item.kind + item.id}><td><button type='button' className='office-resource' onClick={() => openResource(item)}>{item.kind === 'base' || item.kind === 'bitable' ? <TableFile size={19} /> : <FileText size={19} />}<strong>{item.title || item.id}</strong></button></td><td className='office-muted office-small'>{t(item.kind === 'base' || item.kind === 'bitable' ? 'office.resources.filters.base' : 'office.resources.filters.document')}</td><td className='office-muted office-small'>{resourceTime(item)}</td></tr>)}</tbody></table></div> : !hasPermission(hub.status, 'documents.read') ? gate('documents') : <div className='office-empty'><h3>{t(searchTerm ? 'office.resources.notFound' : 'office.resources.firstDocument')}</h3><OfficeButton onClick={() => { setLink(''); setDialog('link'); }}>{t('office.resources.openLink')}</OfficeButton></div>}
      {tab === 'documents' && searchTerm && !hasPermission(hub.status, 'documents.search') && <OfficeButton onClick={() => authorize(scopesFor(hub.status, ['documents.search']))}>{t('office.actions.authorizeRead')}</OfficeButton>}{tab === 'documents' && searchTerm && <div className='office-pagination'><OfficeButton disabled={searchCursors.length === 1 || search.loading} onClick={() => setSearchCursors(value => value.slice(0,-1))}>{t('office.actions.previous')}</OfficeButton><OfficeButton disabled={!search.data?.has_more || !search.data.next_cursor || search.loading} onClick={() => setSearchCursors(value => [...value, search.data!.next_cursor])}>{t('office.actions.more')}</OfficeButton></div>}
    </div>{tab === 'overview' && hasPermission(hub.status, 'calendar.read') && <aside className='office-today'><p className='office-small office-muted'>{new Date().toLocaleDateString()}</p><h3>{t('office.calendar.section')}</h3>{errorPanel(agenda.error)}{agenda.data?.items.slice(0,2).map(item => <div className='office-agenda-item' key={item.id}><time>{item.start ? new Date(item.start).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) : '—'}</time><strong>{item.title}</strong></div>)}{!agenda.loading && !agenda.data?.items.length && <p className='office-small office-muted'>{t('office.calendar.empty')}</p>}<h3 className='office-aside-section'>{t('office.tasks.mine')}</h3>{taskRows(tasks.data?.items.filter(item => !item.completed).slice(0,2) ?? [], true)}<OfficeButton quiet onClick={() => navigate('agenda')}>{t('office.calendar.all')}</OfficeButton></aside>}</div></section>;
  const agendaView = () => <section className='office-content'><div className='office-between'><input className='office-input office-date-input' type='date' aria-label={t('office.calendar.date')} value={date} onChange={event => setDate(event.target.value)} /></div>{!hasPermission(hub.status, 'calendar.read') ? gate('agenda') : <>{errorPanel(agenda.error)}<div className='office-calendar-list'>{agenda.data?.items.map(item => <div className='office-calendar-line' key={item.id}><time>{item.start ? new Date(item.start).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) : '—'} — {item.end ? new Date(item.end).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) : '—'}</time><strong>{item.title}</strong>{item.url && <OfficeButton quiet onClick={() => void openExternal(item.url!)}>{t('office.actions.open')}</OfficeButton>}</div>)}</div>{agenda.loading ? <p>{t('office.loading')}</p> : !agenda.data?.items.length && <p className='office-muted office-small'>{t('office.calendar.empty')}</p>}</>}
    <div className='office-filters office-section-row'>{(['assigned', 'created', 'tasklist'] as const).map(value => <button type='button' key={value} className={relation === value ? 'active' : ''} onClick={() => { setRelation(value); setTaskCursors([null]); }}>{t('office.tasks.' + value)}</button>)}</div>{relation === 'tasklist' && <div className='office-form-row'><label className='office-label'>{t('office.tasks.list')}<select className='office-input' value={tasklistId} onChange={event => { setTasklistId(event.target.value); setTaskCursors([null]); }}><option value=''>{t('office.tasks.chooseList')}</option>{listCache.map(item => <option key={item.id} value={item.id}>{item.title}</option>)}</select></label>{!hasPermission(hub.status, 'tasklists.read') && <OfficeButton onClick={() => authorize(scopesFor(hub.status, ['tasklists.read']))}>{t('office.tasks.readLists')}</OfficeButton>}{lists.data?.has_more && lists.data.next_cursor && <OfficeButton onClick={() => setListCursor(lists.data!.next_cursor)}>{t('office.actions.loadMore')}</OfficeButton>}</div>}
    {!hasPermission(hub.status, 'tasks.read') ? gate('agenda') : <>{errorPanel(tasks.error)}{errorPanel(lists.error)}{taskRows(tasks.data?.items ?? [])}{tasks.loading ? <p>{t('office.loading')}</p> : !tasks.data?.items.length && <p className='office-muted office-small'>{t('office.tasks.empty')}</p>}<div className='office-pagination'><OfficeButton disabled={taskCursors.length === 1} onClick={() => setTaskCursors(value => value.slice(0,-1))}>{t('office.actions.previous')}</OfficeButton><OfficeButton disabled={!tasks.data?.has_more || !tasks.data.next_cursor} onClick={() => setTaskCursors(value => [...value, tasks.data!.next_cursor])}>{t('office.actions.more')}</OfficeButton></div></>}
  </section>;
  const formServices = { status: hub.status, ownerKey: queryOwnerKey, hidden: authVisible, authorize, onClose: () => setDialog(null), onSaved: saved, onResult: showResult, errorPanel };
  const authorization = { flow: hub.flow, busy, error: <>{errorPanel(actionError)}{errorPanel(hub.flowError)}</>, onCancel: () => void cancelAuth(), onOpen: (url: string) => void openExternal(url) };
  const content = !ready && tab !== 'activity' ? <div className='office-empty office-welcome'><h3>{t('office.connection.welcome')}</h3><p>{t('office.connection.welcomeHint')}</p><OfficeButton primary onClick={() => setDialog('connection')}>{t('office.actions.connect')}</OfficeButton></div> : documentId ? !hasPermission(hub.status, 'documents.read') ? gate('documents') : <DocumentView key={documentId} {...formServices} documentId={documentId} onBack={() => navigate('documents')} onOpen={url => void openExternal(url)} refreshKey={refreshKey} onResultId={id => void showOperation(id)} /> : listing ? resourcesView() : tab === 'agenda' ? agendaView() : tab === 'base' ? <BaseView status={hub.status} ownerKey={queryOwnerKey} location={baseLocation} onLocation={setBaseLocation} authorize={() => requestRead('base')} errorPanel={errorPanel} onResultId={id => void showOperation(id)} refreshKey={refreshKey} onEdit={value => { setBaseEdit(value); setDialog('base-edit'); }} /> : <section className='office-content'>{errorPanel(hub.historyError)}<p className='office-small office-muted'>{t('office.operation.historyHint')}</p><div className='office-table-scroll'><table className='office-table'><thead><tr><th>{t('office.operation.action')}</th><th>{t('office.operation.result')}</th><th>{t('office.operation.time')}</th><th /></tr></thead><tbody>{hub.history.map(operation => <tr key={operation.id}><td><strong>{t('office.operation.summary.' + operationKind(operation), { count: operation.items.length, defaultValue: t('office.operation.summary.mixed', { count: operation.items.length }) })}</strong><div className='office-small office-muted'>{operation.actor.name || operation.actor.user_id}</div></td><td><OfficeBadge state={operation.status === 'succeeded' ? 'good' : 'warning'}>{t('office.operation.status.' + operation.status)}</OfficeBadge></td><td className='office-small office-muted'>{new Date(operation.created_at).toLocaleString()}</td><td><OfficeButton quiet onClick={() => void showOperation(operation.id)}>{t('office.actions.details')}</OfficeButton></td></tr>)}</tbody></table></div>{!hub.history.length && <p className='office-empty'>{t(hub.historyLoading ? 'office.loading' : 'office.operation.empty')}</p>}{hub.historyCursor && <OfficeButton onClick={() => void hub.refreshHistory(hub.historyCursor)}>{t('office.actions.loadMore')}</OfficeButton>}</section>;
  return <div className='office-hub'>
    <PageHeader title={t('office.feishu')} className='office-page-header page-header--flush' actions={<>{listing && <><form className='office-header-search' onSubmit={event => { event.preventDefault(); setSearchTerm(searchInput.trim()); setSearchCursors([null]); }}><Search size={14} /><input aria-label={t('office.resources.search')} placeholder={t(tab === 'documents' ? 'office.resources.searchDocuments' : 'office.resources.searchRecent')} value={searchInput} onChange={event => setSearchInput(event.target.value)} /></form><OfficeButton primary onClick={() => newForm('document')}><Add size={14} />{t('office.actions.new')}</OfficeButton></>}{tab === 'agenda' && !documentId && <><OfficeButton onClick={() => newForm('event')}><Add size={14} />{t('office.calendar.section')}</OfficeButton><OfficeButton primary onClick={() => newForm('task')}><Add size={14} />{t('office.tasks.section')}</OfficeButton></>}<span className='office-connection-name' title={hub.status?.actor?.name || hub.status?.connection.name}>{hub.status?.actor?.name || hub.status?.connection.name}</span><OfficeButton quiet disabled={hub.statusLoading || busy} aria-label={t('office.actions.refresh')} onClick={() => { void hub.refreshStatus(); refresh(); }}><Refresh size={15} /></OfficeButton><OfficeButton quiet aria-label={t('office.connection.settings')} onClick={() => setDialog('connection')}><SettingConfig size={16} /></OfficeButton></>} />
    <nav className='office-tabs' role='tablist' aria-label={t('office.title')}>{OFFICE_TABS.map(value => <button type='button' key={value} role='tab' aria-selected={tab === value} onClick={() => navigate(value)}>{t('office.tabs.' + value)}</button>)}</nav>
    <div className='office-body' inert={!!dialog || authVisible}>{errorPanel(hub.statusError)}{!dialog && !authVisible && errorPanel(actionError)}{hub.statusLoading && !hub.status ? <p className='office-empty'>{t('office.loading')}</p> : content}</div>
    {dialog === 'connection' && hub.status && <ConnectionDialog status={hub.status} authorization={authorization} onStart={startAuth} refresh={hub.refreshStatus} onClose={() => setDialog(null)} onError={setActionError} />}
    {(dialog === 'document' || dialog === 'task' || dialog === 'event') && formIdentity && <NativeForm key={formIdentity.key + dialog} {...formServices} kind={dialog} identity={formIdentity} />}
    {dialog === 'base-edit' && baseEdit && <BaseEditForm {...formServices} selection={baseEdit} />}
    {dialog === 'result' && hub.operation && <OperationDialog operation={hub.operation} status={hub.status} busy={busy} error={actionError || hub.operationError ? <>{errorPanel(actionError)}{errorPanel(hub.operationError)}</> : null} onClose={() => { setDialog(null); refresh(); }} onReconcile={() => void run(async () => { hub.setOperation(await officeHub.reconcile(hub.operation!.id)); refresh(); })} onRefresh={() => void showOperation(hub.operation!.id)} onOpen={url => void openExternal(url)} />}
    {dialog === 'link' && <OfficeDialog title={t('office.resources.openLink')} onClose={() => setDialog(null)} footer={<><OfficeButton onClick={() => setDialog(null)}>{t('office.actions.cancel')}</OfficeButton><OfficeButton primary disabled={!link.trim()} onClick={() => { try { if (new URL(link).pathname.startsWith('/base/')) { setBaseLocation(parseBaseLink(link)); navigate('base'); } else navigate('documents', link.trim()); setDialog(null); } catch (error) { setActionError(error); } }}>{t('office.actions.openHere')}</OfficeButton></>}><label className='office-label'>{t('office.resources.link')}<input className='office-input' value={link} onChange={event => setLink(event.target.value)} /></label>{errorPanel(actionError)}</OfficeDialog>}
    {authVisible && <OfficeDialog title={t('office.auth.title')} onClose={closeAuth} busy={busy} footer={<OfficeButton primary disabled={busy} onClick={closeAuth}>{t('office.auth.return')}</OfficeButton>}><AuthorizationPanel {...authorization} /></OfficeDialog>}
  </div>;
}
