import React, { useEffect, useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import {
  Alert,
  Button,
  Checkbox,
  Input,
  Modal,
  Pagination,
  Select,
  Spin,
  type ModalProps,
} from '@arco-design/web-react';
import {
  AddOne,
  ApplicationOne,
  FolderClose,
  Search,
  Pushpin,
  List,
  AllApplication,
} from '@icon-park/react';
import { ipcBridge } from '@/common';
import { uuidv7 } from '@/common/utils/uuidv7';
import type { MiniAppSummary } from '@/common/types/miniAppPlatform';
import { miniAppProduct } from '@/common/adapter/miniAppProductBridge';
import {
  useMiniAppLibrary,
  updateMiniAppWorkspace,
  emptyItem,
  libraryChanged,
  selectLibraryApps,
} from './libraryState';
import { miniAppTrashRequest, miniAppRestoreRequest } from './model';
import MiniAppImportDialog from './MiniAppImportDialog';
import MiniAppCreatorPage from './MiniAppCreatorPage';
import styles from './MiniAppProduct.module.css';

const Dialog = Modal as unknown as React.ComponentType<
  React.PropsWithChildren<ModalProps>
>;
const pageSize = 12;
let lastView = {
  filter: 'all',
  query: '',
  sort: 'recent',
  view: 'grid',
  page: 1,
};

export default function MiniAppLibraryPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { apps, drafts, workspace, loading, failed, refresh } =
    useMiniAppLibrary();
  const [filter, setFilter] = useState(lastView.filter),
    [query, setQuery] = useState(lastView.query);
  const [sort, setSort] = useState(lastView.sort),
    [view, setView] = useState(lastView.view),
    [page, setPage] = useState(lastView.page);
  const [batch, setBatch] = useState(false),
    [selected, setSelected] = useState<Set<string>>(new Set());
  const [dialog, setDialog] = useState<
    'groups' | 'name' | 'move' | 'trash' | 'delete-group' | null
  >(null);
  const [groupId, setGroupId] = useState<string | null>(null),
    [groupName, setGroupName] = useState(''),
    [destination, setDestination] = useState('');
  const [busy, setBusy] = useState(false),
    [error, setError] = useState(''),
    [importing, setImporting] = useState(false);
  const visibleApps = useMemo(
    () => selectLibraryApps(apps, workspace, filter, query, sort),
    [apps, workspace, filter, query, sort],
  );
  const visibleDrafts = drafts.filter(
    (d) =>
      !query.trim() ||
      `${d.name} ${d.description}`
        .toLowerCase()
        .includes(query.trim().toLowerCase()),
  );
  const total = filter === 'drafts' ? visibleDrafts.length : visibleApps.length;
  const currentPage = Math.min(page, Math.max(1, Math.ceil(total / pageSize)));
  const currentApps = visibleApps.slice(
    (currentPage - 1) * pageSize,
    currentPage * pageSize,
  );
  const firstUse = !apps.length && !drafts.length && filter === 'all' && !query;
  const title =
    filter === 'all'
      ? t('miniApps.product.all')
      : filter === 'recent'
        ? t('miniApps.product.recent')
        : filter === 'pinned'
          ? t('miniApps.product.pinned')
          : filter === 'drafts'
            ? t('miniApps.product.drafts')
            : filter === 'trash'
              ? t('miniApps.product.trash')
              : filter === 'unfiled'
                ? t('miniApps.product.unfiled')
                : (workspace.collections.find((c) => c.id === filter)?.name ??
                  t('miniApps.product.all'));
  useEffect(() => {
    lastView = { filter, query, sort, view, page: currentPage };
  }, [filter, query, sort, view, currentPage]);
  useEffect(() => {
    if (
      !loading &&
      !failed &&
      !['all', 'recent', 'pinned', 'drafts', 'trash', 'unfiled'].includes(
        filter,
      ) &&
      !workspace.collections.some((c) => c.id === filter)
    )
      setFilter('all');
  }, [workspace, loading, failed, filter]);
  const choose = (id: string) => {
    setFilter(id);
    setQuery('');
    setPage(1);
    setSelected(new Set());
    setBatch(false);
    setError('');
  };
  const perform = async (action: () => Promise<unknown>, close = true) => {
    if (busy) return;
    setBusy(true);
    setError('');
    try {
      await action();
      if (close) setDialog(null);
      await refresh();
    } catch {
      setError(t('miniApps.product.operationFailed'));
    } finally {
      setBusy(false);
    }
  };
  const move = (ids: Iterable<string>, collectionId: string | null) =>
    updateMiniAppWorkspace((w) => {
      for (const id of ids)
        w.items[id] = {
          ...(w.items[id] ?? emptyItem()),
          collection_id: collectionId,
        };
    });
  const pin = (id: string) =>
    perform(
      () =>
        updateMiniAppWorkspace((w) => {
          const item = w.items[id] ?? emptyItem();
          w.items[id] = { ...item, pinned: !item.pinned };
        }),
      false,
    );
  const open = (app: MiniAppSummary) =>
    navigate(`/mini-apps/${app.miniapp_id}`);
  const groupButton = (
    id: string,
    name: string,
    count: number,
    custom = false,
  ) => (
    <button
      key={id}
      type='button'
      className={styles.collection}
      aria-current={filter === id ? 'page' : undefined}
      onClick={() => choose(id)}
      onDragOver={custom ? (e) => e.preventDefault() : undefined}
      onDrop={
        custom
          ? (e) => {
              e.preventDefault();
              const appId = e.dataTransfer.getData(
                'application/x-nomifun-miniapp',
              );
              if (apps.some((a) => a.miniapp_id === appId))
                void perform(
                  () => move([appId], id === 'unfiled' ? null : id),
                  false,
                );
            }
          : undefined
      }
    >
      <FolderClose size={15} />
      <span>{name}</span>
      <small>{count}</small>
    </button>
  );
  const selectedAction = async (restore: boolean) => {
    const completed: string[] = [];
    try {
      for (const id of selected) {
        const app = apps.find((a) => a.miniapp_id === id);
        if (!app) continue;
        const detail = await ipcBridge.miniapps.getWorkshop.invoke({
          miniapp_id: app.miniapp_id,
        });
        if (restore) {
          const request = miniAppRestoreRequest(detail);
          if (!request) throw new Error('restore unavailable');
          await ipcBridge.miniapps.restore.invoke(request);
        } else {
          const request = miniAppTrashRequest(detail);
          if (!request) throw new Error('trash unavailable');
          await ipcBridge.miniapps.trash.invoke(request);
        }
        completed.push(id);
      }
    } finally {
      setSelected(
        (current) =>
          new Set([...current].filter((id) => !completed.includes(id))),
      );
      libraryChanged();
      await refresh();
    }
  };
  const saveCollection = () => {
    const name = groupName.trim();
    if (
      !name ||
      workspace.collections.some((c) => c.name === name && c.id !== groupId)
    ) {
      setError(t('miniApps.product.collectionNameInvalid'));
      return;
    }
    const id = groupId ?? uuidv7();
    void perform(() =>
      updateMiniAppWorkspace((w) => {
        if (groupId) {
          const group = w.collections.find((c) => c.id === groupId);
          if (!group) throw new Error('missing collection');
          group.name = name;
        } else w.collections.push({ id, name });
      }).then(() => choose(id)),
    );
  };
  if (loading)
    return (
      <div className={styles.empty}>
        <Spin />
        <p>{t('miniApps.product.loading')}</p>
      </div>
    );
  if (failed)
    return (
      <div className={styles.empty}>
        <Alert type='error' content={t('miniApps.product.loadFailed')} />
        <Button onClick={() => void refresh()}>
          {t('miniApps.product.retry')}
        </Button>
      </div>
    );
  return (
    <main className={styles.page}>
      <header className={styles.header}>
        <h1>{t('miniApps.title')}</h1>
        <Button onClick={() => setImporting(true)}>
          {t('miniApps.product.import')}
        </Button>
        <Button
          type='primary'
          icon={<AddOne size={16} />}
          onClick={() => navigate('/mini-apps/new')}
        >
          {t('miniApps.product.create')}
        </Button>
      </header>
      <div className={styles.body}>
        <nav
          className={styles.collections}
          aria-label={t('miniApps.product.collections')}
        >
          {groupButton(
            'all',
            t('miniApps.product.all'),
            selectLibraryApps(apps, workspace, 'all', '', sort).length,
          )}
          {groupButton(
            'recent',
            t('miniApps.product.recent'),
            selectLibraryApps(apps, workspace, 'recent', '', sort).length,
          )}
          {groupButton(
            'pinned',
            t('miniApps.product.pinned'),
            selectLibraryApps(apps, workspace, 'pinned', '', sort).length,
          )}
          {groupButton('drafts', t('miniApps.product.drafts'), drafts.length)}
          <div className={styles.collectionLabel}>
            <span>{t('miniApps.product.collections')}</span>
            <Button
              size='mini'
              type='text'
              icon={<AddOne />}
              aria-label={t('miniApps.product.newCollection')}
              onClick={() => {
                setGroupId(null);
                setGroupName('');
                setError('');
                setDialog('name');
              }}
            />
          </div>
          {workspace.collections.map((c) =>
            groupButton(
              c.id,
              c.name,
              selectLibraryApps(apps, workspace, c.id, '', sort).length,
              true,
            ),
          )}
          {groupButton(
            'unfiled',
            t('miniApps.product.unfiled'),
            selectLibraryApps(apps, workspace, 'unfiled', '', sort).length,
            true,
          )}
          <Button
            type='text'
            onClick={() => {
              setError('');
              setDialog('groups');
            }}
          >
            {t('miniApps.product.manageCollections')}
          </Button>
          <div className={styles.collectionLabel} />
          {groupButton(
            'trash',
            t('miniApps.product.trash'),
            selectLibraryApps(apps, workspace, 'trash', '', sort).length,
          )}
        </nav>
        <section className={styles.content}>
          <div className={styles.title}>
            <h2>{title}</h2>
            <span className={styles.muted}>
              {t('miniApps.product.count', { count: total })}
            </span>
          </div>
          {error && !dialog && (
            <Alert
              type='error'
              content={error}
              closable
              onClose={() => setError('')}
            />
          )}
          <label className={styles.searchBox}>
            <Search />
            <input
              aria-label={t('miniApps.product.search')}
              placeholder={t('miniApps.product.search')}
              value={query}
              onInput={(event) => {
                setQuery(event.currentTarget.value);
                setPage(1);
                setSelected(new Set());
              }}
            />
          </label>
          <div className={styles.toolbar}>
            <Select
              aria-label={t('miniApps.product.sort')}
              style={{ width: 140 }}
              value={sort}
              onChange={setSort}
              options={[
                { value: 'recent', label: t('miniApps.product.recent') },
                { value: 'updated', label: t('miniApps.product.updated') },
                { value: 'name', label: t('miniApps.product.byName') },
              ]}
            />
            <span className={styles.end} />
            <Button
              icon={<AllApplication />}
              aria-pressed={view === 'grid'}
              onClick={() => setView('grid')}
            >
              {t('miniApps.product.grid')}
            </Button>
            <Button
              icon={<List />}
              aria-pressed={view === 'list'}
              onClick={() => setView('list')}
            >
              {t('miniApps.product.list')}
            </Button>
            {filter !== 'drafts' && (
              <Button
                onClick={() => {
                  setBatch(!batch);
                  setSelected(new Set());
                }}
              >
                {t(
                  batch ? 'miniApps.product.done' : 'miniApps.product.organize',
                )}
              </Button>
            )}
          </div>
          {batch && (
            <div className={styles.batch}>
              <span>
                {t('miniApps.product.selected', { count: selected.size })}
              </span>
              <Button
                size='small'
                onClick={() =>
                  setSelected(
                    new Set([
                      ...selected,
                      ...currentApps.map((a) => a.miniapp_id),
                    ]),
                  )
                }
              >
                {t('miniApps.product.selectPage')}
              </Button>
              {filter !== 'trash' && (
                <>
                  <Button
                    size='small'
                    disabled={!selected.size || busy}
                    onClick={() => {
                      setDestination('');
                      setDialog('move');
                    }}
                  >
                    {t('miniApps.product.move')}
                  </Button>
                  <Button
                    size='small'
                    disabled={!selected.size || busy}
                    onClick={() =>
                      void perform(
                        () =>
                          updateMiniAppWorkspace((w) => {
                            for (const id of selected)
                              w.items[id] = {
                                ...(w.items[id] ?? emptyItem()),
                                pinned: true,
                              };
                          }),
                        false,
                      )
                    }
                  >
                    {t('miniApps.product.pin')}
                  </Button>
                </>
              )}
              <Button
                size='small'
                disabled={!selected.size || busy}
                onClick={() =>
                  filter === 'trash'
                    ? void perform(() => selectedAction(true), false)
                    : setDialog('trash')
                }
              >
                {t(
                  filter === 'trash'
                    ? 'miniApps.product.restore'
                    : 'miniApps.product.moveToTrash',
                )}
              </Button>
            </div>
          )}
          <div className={view === 'list' ? styles.list : styles.grid}>
            {firstUse && (
              <div className={styles.firstCreate}>
                <MiniAppCreatorPage embedded />
              </div>
            )}
            {filter === 'drafts'
              ? visibleDrafts
                  .slice((currentPage - 1) * pageSize, currentPage * pageSize)
                  .map((d) => (
                    <article key={d.id} className={styles.card}>
                      <button
                        className={styles.open}
                        onClick={() => navigate(`/mini-apps/create/${d.id}`)}
                      >
                        <span className={styles.icon}>
                          <ApplicationOne />
                        </span>
                        <strong>
                          {d.name || t('miniApps.product.untitled')}
                        </strong>
                      </button>
                      <p>
                        {t(
                          d.status === 'generating'
                            ? 'miniApps.product.generating'
                            : 'miniApps.product.draftSaved',
                        )}
                      </p>
                      <div className={styles.cardFooter}>
                        <Button
                          type='text'
                          onClick={() => navigate(`/mini-apps/create/${d.id}`)}
                        >
                          {t('miniApps.product.continue')}
                        </Button>
                        <Button
                          type='text'
                          disabled={busy}
                          onClick={() =>
                            void perform(() =>
                              miniAppProduct.discard
                                .invoke({
                                  id: d.id,
                                  expected_revision: d.revision,
                                })
                                .then(libraryChanged),
                            )
                          }
                        >
                          {t('miniApps.product.discardDraft')}
                        </Button>
                      </div>
                    </article>
                  ))
              : currentApps.map((app) => {
                  const item = workspace.items[app.miniapp_id] ?? emptyItem();
                  const name = item.name ?? app.display_name;
                  return (
                    <article
                      key={app.miniapp_id}
                      className={`${styles.card} ${selected.has(app.miniapp_id) ? styles.selected : ''}`}
                      draggable={!batch && filter !== 'trash'}
                      onDragStart={(e) =>
                        e.dataTransfer.setData(
                          'application/x-nomifun-miniapp',
                          app.miniapp_id,
                        )
                      }
                    >
                      <div className={styles.cardTop}>
                        {batch && (
                          <Checkbox
                            aria-label={`${t('miniApps.product.select')} ${name}`}
                            checked={selected.has(app.miniapp_id)}
                            onChange={(checked) =>
                              setSelected((current) => {
                                const next = new Set(current);
                                if (checked) next.add(app.miniapp_id);
                                else next.delete(app.miniapp_id);
                                return next;
                              })
                            }
                          />
                        )}
                        <button
                          type='button'
                          className={styles.open}
                          onClick={() => open(app)}
                          aria-label={`${t('miniApps.product.open')} ${name}`}
                        >
                          <span className={styles.icon}>
                            <ApplicationOne size={17} />
                          </span>
                          <strong>{name}</strong>
                        </button>
                        {filter !== 'trash' && (
                          <Button
                            type='text'
                            size='mini'
                            icon={
                              <Pushpin
                                theme={item.pinned ? 'filled' : 'outline'}
                              />
                            }
                            disabled={busy}
                            aria-label={`${t(item.pinned ? 'miniApps.product.unpin' : 'miniApps.product.pin')} ${name}`}
                            onClick={() => void pin(app.miniapp_id)}
                          />
                        )}
                      </div>
                      <p>
                        {app.description || t('miniApps.product.noDescription')}
                      </p>
                      <div className={styles.cardFooter}>
                        <span>
                          {workspace.collections.find(
                            (c) => c.id === item.collection_id,
                          )?.name ?? t('miniApps.product.unfiled')}
                        </span>
                        <span>
                          {app.lifecycle === 'disabled'
                            ? t('miniApps.product.disabled')
                            : item.last_opened
                              ? new Date(item.last_opened).toLocaleDateString()
                              : t('miniApps.product.ready')}
                        </span>
                      </div>
                    </article>
                  );
                })}
            {!total && !firstUse && (
              <div className={styles.empty}>
                <h3>
                  {t(
                    query
                      ? 'miniApps.product.noMatch'
                      : 'miniApps.product.empty',
                  )}
                </h3>
                <p>
                  {t(
                    query
                      ? 'miniApps.product.noMatchHint'
                      : 'miniApps.product.emptyHint',
                  )}
                </p>
                <Button
                  type='primary'
                  onClick={() =>
                    query ? choose('all') : navigate('/mini-apps/new')
                  }
                >
                  {t(
                    query ? 'miniApps.product.all' : 'miniApps.product.create',
                  )}
                </Button>
              </div>
            )}
          </div>
          <div className={styles.pagination}>
            <span className={styles.muted}>
              {t('miniApps.product.count', { count: total })}
            </span>
            <Pagination
              current={currentPage}
              total={total}
              pageSize={pageSize}
              onChange={setPage}
              size='small'
            />
          </div>
        </section>
      </div>
      <Dialog
        visible={dialog !== null}
        title={t(
          dialog === 'name'
            ? groupId
              ? 'miniApps.product.renameCollection'
              : 'miniApps.product.newCollection'
            : dialog === 'move'
              ? 'miniApps.product.move'
              : dialog === 'trash'
                ? 'miniApps.product.moveToTrash'
                : dialog === 'delete-group'
                  ? 'miniApps.product.deleteCollection'
                  : 'miniApps.product.manageCollections',
        )}
        onCancel={busy ? undefined : () => setDialog(null)}
        footer={dialog === 'groups' ? null : undefined}
        confirmLoading={busy}
        okText={t('miniApps.product.confirm')}
        cancelText={t('miniApps.product.cancel')}
        onOk={() => {
          if (dialog === 'name') saveCollection();
          else if (dialog === 'move')
            void perform(() =>
              move(selected, destination || null).then(() =>
                setSelected(new Set()),
              ),
            );
          else if (dialog === 'trash')
            void perform(() => selectedAction(false));
          else if (dialog === 'delete-group')
            void perform(() =>
              updateMiniAppWorkspace((w) => {
                w.collections = w.collections.filter((c) => c.id !== groupId);
                for (const item of Object.values(w.items))
                  if (item.collection_id === groupId) item.collection_id = null;
              }),
            );
        }}
      >
        {dialog === 'name' && (
          <Input
            autoFocus
            aria-label={t('miniApps.product.collectionName')}
            placeholder={t('miniApps.product.collectionName')}
            maxLength={60}
            value={groupName}
            onChange={setGroupName}
            onPressEnter={saveCollection}
          />
        )}
        {dialog === 'move' && (
          <select
            className={styles.select}
            aria-label={t('miniApps.product.collection')}
            value={destination}
            onChange={(event) => setDestination(event.currentTarget.value)}
          >
            <option value=''>{t('miniApps.product.unfiled')}</option>
            {workspace.collections.map((c) => (
              <option key={c.id} value={c.id}>
                {c.name}
              </option>
            ))}
          </select>
        )}
        {dialog === 'trash' && <p>{t('miniApps.product.trashHint')}</p>}
        {dialog === 'delete-group' && (
          <p>{t('miniApps.product.deleteCollectionHint')}</p>
        )}
        {dialog === 'groups' && (
          <>
            {workspace.collections.map((c) => (
              <div className={styles.groupRow} key={c.id}>
                <FolderClose />
                <strong>{c.name}</strong>
                <Button
                  type='text'
                  onClick={() => {
                    setGroupId(c.id);
                    setGroupName(c.name);
                    setDialog('name');
                  }}
                >
                  {t('miniApps.product.rename')}
                </Button>
                <Button
                  type='text'
                  onClick={() => {
                    setGroupId(c.id);
                    setDialog('delete-group');
                  }}
                >
                  {t('miniApps.product.delete')}
                </Button>
              </div>
            ))}
            <Button
              style={{ marginTop: 16 }}
              onClick={() => {
                setGroupId(null);
                setGroupName('');
                setDialog('name');
              }}
            >
              {t('miniApps.product.newCollection')}
            </Button>
          </>
        )}
        {error && <Alert type='error' content={error} />}
      </Dialog>
      <MiniAppImportDialog
        visible={importing}
        onClose={() => setImporting(false)}
        onOpened={(id) => {
          setImporting(false);
          libraryChanged();
          navigate(`/mini-apps/${id}`);
        }}
      />
    </main>
  );
}
