import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Alert, Button, Dropdown, Input, Menu, Modal, Select, Spin, Switch } from '@arco-design/web-react';
import { AddOne, ArrowRight, Delete, EditTwo, FolderClose, MoreOne, Pushpin, Refresh, Search, Upload } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import { useNavigate, useSearchParams } from 'react-router-dom';
import { pluginPlatform } from '@/common/adapter/pluginPlatformBridge';
import type { PluginDraftSummary, PluginLibraryItem, PluginLibraryState, PluginSummary } from '@/common/types/pluginPlatform';
import type { PluginAuthoringSessionSummary } from '@/common/types/pluginDevelopment';
import { isDesktopShell } from '@/renderer/utils/platform';
import PluginImportDialog from './PluginImportDialog';
import PluginWorkspace, { PluginVisual } from './PluginWorkspace';
import { subscribePluginLibraryChanges, notifyPluginLibraryChanged, updatePluginLibraryState } from './pluginLibraryState';
import { isPluginLibraryView, pluginLibraryCounts, pluginShape } from './pluginPlatformModel';
import { launchPluginAuthoring } from './pluginAuthoringLaunch';
import styles from './PluginPlatform.module.css';

type LibrarySort = 'recent' | 'name';
type ItemEditor = { plugin: PluginSummary; name: string; collection: string };
type CreationRecord = { key: string; name: string; description: string; updated: number; running: boolean; session?: string; draft?: string };

export default function PluginLibraryPage() {
  const { t, i18n } = useTranslation();
  const navigate = useNavigate();
  const [searchParams] = useSearchParams();
  const [plugins, setPlugins] = useState<PluginSummary[]>([]);
  const [drafts, setDrafts] = useState<PluginDraftSummary[]>([]);
  const [sessions, setSessions] = useState<PluginAuthoringSessionSummary[]>([]);
  const [sessionsFailed, setSessionsFailed] = useState(false);
  const [organization, setOrganization] = useState<PluginLibraryState>({ revision: 0, collections: [], items: [] });
  const [query, setQuery] = useState('');
  const [sort, setSort] = useState<LibrarySort>('recent');
  const [collection, setCollection] = useState('');
  const [status, setStatus] = useState<'all' | 'pinned' | 'disabled'>('all');
  const [loading, setLoading] = useState(true);
  const [busyId, setBusyId] = useState('');
  const [error, setError] = useState('');
  const [importVisible, setImportVisible] = useState(false);
  const [editor, setEditor] = useState<ItemEditor | null>(null);
  const [trashTarget, setTrashTarget] = useState<PluginSummary | null>(null);
  const [categoriesVisible, setCategoriesVisible] = useState(false);
  const [categoryNames, setCategoryNames] = useState<Record<string, string>>({});
  const loadSequence = useRef(0);
  const desktopShell = isDesktopShell();
  const requestedView = searchParams.get('view');
  const view = isPluginLibraryView(requestedView) ? requestedView : 'all';
  const recordsView = view === 'drafts';
  const trashView = view === 'trash';

  const refresh = useCallback(async () => {
    const request = ++loadSequence.current;
    try {
      const [library, draftList, state, sessionList] = await Promise.all([
        pluginPlatform.plugins.list.invoke(), pluginPlatform.drafts.list.invoke(), pluginPlatform.libraryState.get.invoke(),
        desktopShell ? pluginPlatform.authoring.listSessions.invoke().catch(() => null) : Promise.resolve({ sessions: [] }),
      ]);
      if (request !== loadSequence.current) return;
      setPlugins(library.plugins); setDrafts(draftList.drafts); setOrganization(state);
      if (sessionList) setSessions(sessionList.sessions);
      setSessionsFailed(sessionList === null); setError('');
    } catch (caught) {
      if (request === loadSequence.current) setError(t('pluginPlatform.library.loadFailed'));
      console.error('[pluginPlatform] library load failed', caught);
    } finally { if (request === loadSequence.current) setLoading(false); }
  }, [t, desktopShell]);

  useEffect(() => {
    void refresh();
    const off = subscribePluginLibraryChanges(refresh);
    return () => { loadSequence.current += 1; off(); };
  }, [refresh]);
  useEffect(() => { if (desktopShell && searchParams.get('import') === '1') setImportVisible(true); }, [desktopShell, searchParams]);
  useEffect(() => { setCollection(''); setStatus('all'); setQuery(''); }, [view]);
  useEffect(() => { if (collection && !organization.collections.includes(collection)) setCollection(''); }, [collection, organization.collections]);

  const items = useMemo(() => new Map(organization.items.map(item => [item.plugin_id, item])), [organization]);
  const nameOf = (plugin: PluginSummary) => items.get(plugin.plugin_id)?.custom_name || plugin.display_name;
  const normalized = query.trim().toLocaleLowerCase();
  const visiblePlugins = plugins.filter(plugin => {
    if ((plugin.trashed_at_ms !== undefined) !== trashView) return false;
    if (!trashView && view !== 'all' && !recordsView && pluginShape(plugin) !== view) return false;
    const item = items.get(plugin.plugin_id);
    if (!trashView && collection && item?.collection_id !== collection) return false;
    if (!trashView && status === 'pinned' && !item?.pinned) return false;
    if (!trashView && status === 'disabled' && plugin.enabled) return false;
    return !normalized || [nameOf(plugin), plugin.description, plugin.package_id, item?.collection_id].join(' ').toLocaleLowerCase().includes(normalized);
  }).sort((a, b) => sort === 'name' ? nameOf(a).localeCompare(nameOf(b), i18n.language) : b.updated_at_ms - a.updated_at_ms);

  const records = useMemo(() => {
    const bySession = new Map<string, PluginDraftSummary[]>();
    for (const draft of drafts) if (draft.source_conversation_id) {
      const list = bySession.get(draft.source_conversation_id) ?? [];
      list.push(draft); bySession.set(draft.source_conversation_id, list);
    }
    const sessionIds = new Set(sessions.map(session => session.conversation_id));
    const result: CreationRecord[] = sessions.map(session => {
      const artifacts = bySession.get(session.conversation_id) ?? [];
      return { key: session.conversation_id, name: session.name || t('pluginPlatform.library.untitled'),
        description: artifacts.map(draft => draft.display_name + ' ' + draft.description).join(' · '),
        updated: Math.max(session.modified_at, ...artifacts.map(draft => draft.updated_at_ms)), running: !!session.runtime?.is_processing, session: session.conversation_id };
    });
    for (const draft of drafts) if (!draft.source_conversation_id || !sessionIds.has(draft.source_conversation_id)) {
      result.push({ key: draft.draft_id, name: draft.display_name || t('pluginPlatform.library.untitled'), description: draft.description,
        updated: draft.updated_at_ms, running: false, draft: draft.draft_id });
    }
    return result;
  }, [drafts, sessions, t]);
  const visibleRecords = records.filter(record => !normalized || (record.name + ' ' + record.description).toLocaleLowerCase().includes(normalized))
    .sort((a, b) => sort === 'name' ? a.name.localeCompare(b.name, i18n.language) : b.updated - a.updated);
  const counts = pluginLibraryCounts(plugins, records.length);

  const perform = async (id: string, operation: () => Promise<unknown>) => {
    if (!desktopShell || busyId) return false;
    setBusyId(id); setError('');
    try { await operation(); notifyPluginLibraryChanged(); await refresh(); return true; }
    catch (caught) { console.error('[pluginPlatform] library update failed', caught); setError(t('pluginPlatform.library.mutationFailed')); return false; }
    finally { setBusyId(''); }
  };
  const updateItem = (plugin: PluginSummary, update: (item: PluginLibraryItem) => void) => perform(plugin.plugin_id, () =>
    updatePluginLibraryState(state => {
      let item = state.items.find(value => value.plugin_id === plugin.plugin_id);
      if (!item) { item = { plugin_id: plugin.plugin_id, pinned: false }; state.items.push(item); }
      update(item);
      state.collections = [...new Set(state.items.flatMap(value => value.collection_id ? [value.collection_id] : []))];
    }));
  const openPlugin = (plugin: PluginSummary) => {
    navigate('/plugins/run/' + encodeURIComponent(plugin.plugin_id));
  };
  const create = () => { void launchPluginAuthoring(navigate).catch(() => setError(t('pluginPlatform.creator.launchFailed'))); };
  const editPlugin = (plugin: PluginSummary) => { void launchPluginAuthoring(navigate, { plugin_id: plugin.plugin_id, expected_plugin_revision: plugin.revision })
    .catch(() => setError(t('pluginPlatform.creator.launchFailed'))); };
  const openRecord = (record: CreationRecord) => {
    if (record.session) navigate('/plugins/authoring/' + encodeURIComponent(record.session));
    else void launchPluginAuthoring(navigate, { draft_id: record.draft }).catch(() => setError(t('pluginPlatform.creator.launchFailed')));
  };
  const saveEditor = async () => {
    if (!editor || !editor.name.trim()) return;
    const saved = await updateItem(editor.plugin, item => {
      item.custom_name = editor.name.trim() === editor.plugin.display_name ? undefined : editor.name.trim();
      item.collection_id = editor.collection.trim() || undefined;
    });
    if (saved) setEditor(null);
  };
  const title = t('pluginPlatform.library.views.' + view + '.title');
  const filtered = !!normalized || !!collection || status !== 'all';

  return <PluginWorkspace activeView={view} counts={counts}>
    <main className={styles.page}>
      <header className={styles.pageHeader}>
        <div className={styles.headerCopy}>
          <span className={styles.eyebrow}>{t('pluginPlatform.library.eyebrow')}</span>
          <h1>{title}</h1>
          <p>{t('pluginPlatform.library.views.' + view + '.subtitle', { count: counts[view] })}</p>
        </div>
        {desktopShell && <div className={styles.headerActions}>
          {!trashView && <Button icon={<Upload />} onClick={() => setImportVisible(true)}>{t('pluginPlatform.actions.import')}</Button>}
          <Button type='primary' icon={<AddOne />} onClick={create}>{t('pluginPlatform.actions.create')}</Button>
        </div>}
      </header>

      {error && <Alert type='error' content={error} showIcon />}
      {!desktopShell && <Alert type='info' content={t('pluginPlatform.readOnly.body')} showIcon />}
      {recordsView && sessionsFailed && <Alert type='warning' content={t('pluginPlatform.authoring.tasksLoadFailed')} action={<Button type='text' onClick={() => void refresh()}>{t('pluginPlatform.actions.refresh')}</Button>} />}
      <div className={styles.libraryToolbar}>
        <Input className={styles.librarySearch} value={query} prefix={<Search />} allowClear
          placeholder={t(recordsView ? 'pluginPlatform.library.searchRecords' : 'pluginPlatform.library.search')}
          aria-label={t(recordsView ? 'pluginPlatform.library.searchRecords' : 'pluginPlatform.library.search')} onChange={setQuery} />
        <div className={styles.toolbarActions}>
          {!recordsView && !trashView && <Select className={styles.filterSelect} value={status}
            aria-label={t('pluginPlatform.library.filterLabel')} onChange={value => setStatus(value)}
            options={['all', 'pinned', 'disabled'].map(value => ({ value, label: t('pluginPlatform.library.filters.' + value) }))} />}
          <Select className={styles.sortSelect} value={sort} aria-label={t('pluginPlatform.library.sortLabel')}
            onChange={value => setSort(value)} options={[
              { value: 'recent', label: t('pluginPlatform.library.sortRecent') }, { value: 'name', label: t('pluginPlatform.library.sortName') },
            ]} />
          <Button icon={<Refresh />} loading={loading} aria-label={t('pluginPlatform.actions.refresh')} title={t('pluginPlatform.actions.refresh')} onClick={() => void refresh()} />
        </div>
      </div>

      {!recordsView && !trashView && <div className={styles.libraryFilters}>
        <div className={styles.typeFilters}>
          {(['all', 'ui_only', 'headless', 'mixed'] as const).map(type => <button type='button' key={type}
            aria-pressed={view === type} className={view === type ? styles.filterActive : ''}
            onClick={() => navigate(type === 'all' ? '/plugins' : '/plugins?view=' + type)}>
            {t(type === 'all' ? 'pluginPlatform.library.allTypes' : 'pluginPlatform.workspace.views.' + type)}<span>{counts[type]}</span>
          </button>)}
        </div>
        {!!organization.collections.length && <div className={styles.collectionFilters}>
          <FolderClose />
          <button type='button' aria-pressed={!collection} className={!collection ? styles.collectionActive : ''} onClick={() => setCollection('')}>{t('pluginPlatform.library.allCategories')}</button>
          {organization.collections.map(category => <button type='button' key={category} aria-pressed={category === collection}
            className={category === collection ? styles.collectionActive : ''} onClick={() => setCollection(category)}>{category}</button>)}
          {desktopShell && <Button type='text' size='small' icon={<EditTwo />} aria-label={t('pluginPlatform.library.manageCategories')} onClick={() => {
            setError(''); setCategoryNames(Object.fromEntries(organization.collections.map(category => [category, category]))); setCategoriesVisible(true);
          }} />}
        </div>}
      </div>}

      {loading ? <div className={styles.emptyState}><Spin /><p>{t('pluginPlatform.library.loading')}</p></div>
        : recordsView && visibleRecords.length ? <section className={styles.creationRecords} aria-label={title}>
          {visibleRecords.map(record => <article key={record.key} className={styles.creationRecord}>
            <PluginVisual draft />
            <div><strong>{record.name}</strong><p>{record.description || t('pluginPlatform.library.recordDescription')}</p>
              <small>{new Date(record.updated).toLocaleString(i18n.language)}</small></div>
            {record.running && <span className={styles.runningLabel}><i />{t('pluginPlatform.authoring.taskRunning')}</span>}
            <Button disabled={!desktopShell} icon={<ArrowRight />} onClick={() => openRecord(record)}>{t('pluginPlatform.library.continue')}</Button>
          </article>)}
        </section>
        : !recordsView && visiblePlugins.length ? <section className={styles.pluginGrid} aria-label={title}>
          {visiblePlugins.map(plugin => {
            const item = items.get(plugin.plugin_id);
            const name = nameOf(plugin);
            const shape = pluginShape(plugin);
            const busy = busyId === plugin.plugin_id;
            return <article key={plugin.plugin_id} className={styles.pluginCard}>
              <div className={styles.cardTop}>
                <PluginVisual shape={shape} />
                {item?.pinned && <Pushpin className={styles.pinMark} theme='filled' />}
                {desktopShell && <Dropdown trigger='click' position='br' droplist={<Menu onClickMenuItem={key => {
                  if (key === 'edit') editPlugin(plugin);
                  if (key === 'organize') { setError(''); setEditor({ plugin, name, collection: item?.collection_id ?? '' }); }
                  if (key === 'pin') void updateItem(plugin, next => { next.pinned = !next.pinned; });
                  if (key === 'trash') setTrashTarget(plugin);
                  if (key === 'restore') void perform(plugin.plugin_id, () => pluginPlatform.plugins.restore.invoke({ plugin_id: plugin.plugin_id, request: { expected_revision: plugin.revision } }));
                }}>
                  {!trashView && <Menu.Item key='edit'>{t('pluginPlatform.actions.edit')}</Menu.Item>}
                  {!trashView && <Menu.Item key='organize'>{t('pluginPlatform.library.organize')}</Menu.Item>}
                  {!trashView && <Menu.Item key='pin'>{t(item?.pinned ? 'pluginPlatform.library.unpin' : 'pluginPlatform.library.pin')}</Menu.Item>}
                  {trashView ? <Menu.Item key='restore'>{t('pluginPlatform.actions.restoreTrash')}</Menu.Item> : <Menu.Item key='trash'>{t('pluginPlatform.actions.trash')}</Menu.Item>}
                </Menu>}>
                  <Button type='text' icon={<MoreOne />} disabled={!!busyId} aria-label={t('pluginPlatform.library.moreActions', { name })} />
                </Dropdown>}
              </div>
              <button type='button' className={styles.cardIdentity} onClick={() => openPlugin(plugin)}><strong>{name}</strong></button>
              <p className={styles.cardDescription}>{plugin.description || t('pluginPlatform.library.defaultDescription')}</p>
              <div className={styles.cardTags}><span>{t('pluginPlatform.shape.' + shape)}</span>{item?.collection_id && <span>{item.collection_id}</span>}</div>
              {plugin.last_error && <p className={styles.cardError} title={plugin.last_error}>{plugin.last_error}</p>}
              <footer className={styles.cardFooter}>
                <Button type='text' onClick={() => openPlugin(plugin)}>{t(trashView ? 'pluginPlatform.library.viewDetails' : plugin.has_ui ? 'pluginPlatform.actions.open' : 'pluginPlatform.library.viewCapabilities')}<ArrowRight /></Button>
                {!trashView && <span className={styles.cardToggle}><span>{t(plugin.enabled ? 'pluginPlatform.actions.enabled' : 'pluginPlatform.actions.disabled')}</span>
                  <Switch size='small' disabled={!desktopShell || (!!busyId && !busy)} loading={busy} checked={plugin.enabled}
                    aria-label={t('pluginPlatform.library.toggle', { name })} onChange={enabled => void perform(plugin.plugin_id, () => pluginPlatform.plugins.setEnabled.invoke({
                      plugin_id: plugin.plugin_id, request: { expected_revision: plugin.revision, enabled },
                    }))} /></span>}
              </footer>
            </article>;
          })}
        </section> : <div className={styles.emptyState}>
          <PluginVisual draft={recordsView} large />
          <h2>{t(normalized || filtered ? 'pluginPlatform.library.noResultsTitle' : trashView ? 'pluginPlatform.library.trashEmptyTitle' : recordsView ? 'pluginPlatform.library.recordsEmptyTitle' : 'pluginPlatform.library.emptyTitle')}</h2>
          <p>{t(normalized || filtered ? 'pluginPlatform.library.noResultsBody' : trashView ? 'pluginPlatform.library.trashEmptyBody' : recordsView ? 'pluginPlatform.library.recordsEmptyBody' : 'pluginPlatform.library.emptyBody')}</p>
          {filtered ? <Button onClick={() => { setQuery(''); setCollection(''); setStatus('all'); }}>{t('pluginPlatform.library.clearSearch')}</Button>
            : desktopShell && !trashView && <Button type='primary' icon={<AddOne />} onClick={create}>{t('pluginPlatform.library.startCreating')}</Button>}
        </div>}

      {desktopShell && <PluginImportDialog visible={importVisible} onCancel={() => {
        setImportVisible(false); if (searchParams.has('import')) navigate('/plugins', { replace: true });
      }} onInstalled={detail => { setImportVisible(false); notifyPluginLibraryChanged(); openPlugin(detail.summary); }} />}
      <Modal visible={!!editor} title={t('pluginPlatform.library.organize')} confirmLoading={!!busyId} okText={t('pluginPlatform.actions.save')} cancelText={t('pluginPlatform.actions.cancel')}
        okButtonProps={{ disabled: !editor?.name.trim() }} onOk={() => void saveEditor()} onCancel={() => setEditor(null)} unmountOnExit>
        {editor && <div className={styles.modalBody}>
          {error && <Alert type='error' content={error} showIcon />}
          <label><span>{t('pluginPlatform.library.nameLabel')}</span><Input value={editor.name} onChange={name => setEditor({ ...editor, name })} /></label>
          <label><span>{t('pluginPlatform.library.categoryLabel')}</span>
            <input className={styles.categoryInput} list='plugin-collections' value={editor.collection}
              onChange={event => setEditor({ ...editor, collection: event.target.value })} placeholder={t('pluginPlatform.library.categoryPlaceholder')} />
            <datalist id='plugin-collections'>{organization.collections.map(category => <option key={category} value={category} />)}</datalist>
          </label><p className={styles.muted}>{t('pluginPlatform.library.categoryHint')}</p>
        </div>}
      </Modal>
      <Modal visible={categoriesVisible} title={t('pluginPlatform.library.manageCategories')} confirmLoading={!!busyId} onCancel={() => setCategoriesVisible(false)} okText={t('pluginPlatform.actions.save')} cancelText={t('pluginPlatform.actions.cancel')}
        onOk={() => void perform('categories', async () => {
          await updatePluginLibraryState(state => {
            for (const item of state.items) if (item.collection_id && Object.hasOwn(categoryNames, item.collection_id)) item.collection_id = categoryNames[item.collection_id]?.trim() || undefined;
            state.collections = [...new Set(state.items.flatMap(item => item.collection_id ? [item.collection_id] : []))];
          }); setCategoriesVisible(false);
        })} unmountOnExit>
        <div className={styles.modalBody}><p>{t('pluginPlatform.library.manageCategoriesHint')}</p>
          {error && <Alert type='error' content={error} showIcon />}
          {organization.collections.map(category => <label key={category} className={styles.categoryEditor}><FolderClose />
            <Input aria-label={category} value={categoryNames[category] ?? ''} onChange={name => setCategoryNames(current => ({ ...current, [category]: name }))} />
            <Button type='text' status='danger' icon={<Delete />} aria-label={t('pluginPlatform.library.removeCategory', { name: category })} onClick={() => setCategoryNames(current => ({ ...current, [category]: '' }))} />
          </label>)}
        </div>
      </Modal>
      <Modal visible={!!trashTarget} title={t('pluginPlatform.dialog.trash.title')} confirmLoading={!!busyId} onCancel={() => setTrashTarget(null)} okText={t('pluginPlatform.actions.trash')} cancelText={t('pluginPlatform.actions.cancel')}
        okButtonProps={{ status: 'danger' }} onOk={() => { if (trashTarget) void perform(trashTarget.plugin_id, async () => {
          await pluginPlatform.plugins.trash.invoke({ plugin_id: trashTarget.plugin_id, request: { expected_revision: trashTarget.revision } }); setTrashTarget(null);
        }); }}><div className={styles.modalBody}><p>{t('pluginPlatform.dialog.trash.body')}</p>{error && <Alert type='error' content={error} showIcon />}</div></Modal>
    </main>
  </PluginWorkspace>;
}
