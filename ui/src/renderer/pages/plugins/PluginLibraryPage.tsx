import { useCallback, useEffect, useMemo, useState } from 'react';
import { Alert, Button, Dropdown, Input, Menu, Select, Spin, Switch } from '@arco-design/web-react';
import {
  AddOne,
  Attention,
  MoreOne,
  PreviewOpen,
  Pushpin,
  Refresh,
  Search,
} from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import { useNavigate, useSearchParams } from 'react-router-dom';
import { pluginPlatform } from '@/common/adapter/pluginPlatformBridge';
import type {
  PluginDraftSummary,
  PluginLibraryState,
  PluginSummary,
} from '@/common/types/pluginPlatform';
import { isDesktopShell } from '@/renderer/utils/platform';
import PluginImportDialog from './PluginImportDialog';
import PluginWorkspace, { PluginVisual } from './PluginWorkspace';
import {
  PLUGIN_LIBRARY_CHANGED,
  notifyPluginLibraryChanged,
  updatePluginLibraryState,
} from './pluginLibraryState';
import {
  draftNeedsAttention,
  isPluginLibraryView,
  pluginEntryMatchesView,
  pluginLibraryCounts,
  pluginLibraryEntries,
  pluginNeedsAttention,
  pluginShape,
} from './pluginPlatformModel';
import styles from './PluginPlatform.module.css';

type LibrarySort = 'recent' | 'name';

export default function PluginLibraryPage() {
  const { t, i18n } = useTranslation();
  const navigate = useNavigate();
  const [searchParams] = useSearchParams();
  const [plugins, setPlugins] = useState<PluginSummary[]>([]);
  const [drafts, setDrafts] = useState<PluginDraftSummary[]>([]);
  const [organization, setOrganization] = useState<PluginLibraryState>({
    revision: 0,
    collections: [],
    items: [],
  });
  const [query, setQuery] = useState('');
  const [sort, setSort] = useState<LibrarySort>('recent');
  const [loading, setLoading] = useState(true);
  const [busyId, setBusyId] = useState('');
  const [error, setError] = useState('');
  const [importVisible, setImportVisible] = useState(false);
  const desktopShell = isDesktopShell();
  const requestedView = searchParams.get('view');
  const view = isPluginLibraryView(requestedView) ? requestedView : 'all';

  const refresh = useCallback(async () => {
    try {
      const [library, draftList, state] = await Promise.all([
        pluginPlatform.plugins.list.invoke(),
        pluginPlatform.drafts.list.invoke(),
        pluginPlatform.libraryState.get.invoke(),
      ]);
      setPlugins(library.plugins);
      setDrafts(draftList.drafts);
      setOrganization(state);
      setError('');
    } catch (caught) {
      console.error('[pluginPlatform] library load failed', caught);
      setError(t('pluginPlatform.library.loadFailed'));
    } finally {
      setLoading(false);
    }
  }, [t]);

  useEffect(() => {
    void refresh();
    window.addEventListener(PLUGIN_LIBRARY_CHANGED, refresh);
    return () => window.removeEventListener(PLUGIN_LIBRARY_CHANGED, refresh);
  }, [refresh]);

  useEffect(() => {
    if (desktopShell && searchParams.get('import') === '1') setImportVisible(true);
  }, [desktopShell, searchParams]);

  const entries = useMemo(() => {
    const normalized = query.trim().toLocaleLowerCase();
    const visible = pluginLibraryEntries(plugins, drafts).filter((entry) => {
      if (!pluginEntryMatchesView(entry, view)) return false;
      if (!normalized) return true;
      const text = entry.kind === 'plugin'
        ? `${entry.plugin.display_name} ${entry.plugin.description} ${entry.plugin.package_id}`
        : `${entry.draft.display_name} ${entry.draft.description} ${entry.draft.package_id ?? ''}`;
      return text.toLocaleLowerCase().includes(normalized);
    });
    if (sort === 'name') {
      return [...visible].sort((left, right) => {
        const leftName = left.kind === 'plugin' ? left.plugin.display_name : left.draft.display_name;
        const rightName = right.kind === 'plugin' ? right.plugin.display_name : right.draft.display_name;
        return leftName.localeCompare(rightName, i18n.language);
      });
    }
    return visible;
  }, [drafts, i18n.language, plugins, query, sort, view]);

  const counts = useMemo(() => pluginLibraryCounts(plugins, drafts), [drafts, plugins]);
  const pinned = useMemo(
    () => new Set(organization.items.filter((item) => item.pinned).map((item) => item.plugin_id)),
    [organization.items],
  );

  const toggleEnabled = async (plugin: PluginSummary, enabled: boolean) => {
    if (!desktopShell) return;
    setBusyId(plugin.plugin_id);
    try {
      await pluginPlatform.plugins.setEnabled.invoke({
        plugin_id: plugin.plugin_id,
        request: { expected_revision: plugin.revision, enabled },
      });
      notifyPluginLibraryChanged();
      await refresh();
    } catch (caught) {
      console.error('[pluginPlatform] enable update failed', caught);
      setError(t('pluginPlatform.library.mutationFailed'));
    } finally {
      setBusyId('');
    }
  };

  const togglePinned = async (plugin: PluginSummary) => {
    if (!desktopShell) return;
    setBusyId(plugin.plugin_id);
    try {
      const next = await updatePluginLibraryState((state) => {
        const index = state.items.findIndex((item) => item.plugin_id === plugin.plugin_id);
        if (index < 0) state.items.push({ plugin_id: plugin.plugin_id, pinned: true });
        else state.items[index] = { ...state.items[index]!, pinned: !state.items[index]!.pinned };
      });
      setOrganization(next);
    } catch (caught) {
      console.error('[pluginPlatform] pin update failed', caught);
      setError(t('pluginPlatform.library.mutationFailed'));
    } finally {
      setBusyId('');
    }
  };

  const pageTitle = t(`pluginPlatform.library.views.${view}.title`);
  const pageSubtitle = t(`pluginPlatform.library.views.${view}.subtitle`, { count: counts[view] });

  return (
    <PluginWorkspace activeView={view} counts={counts} onImport={() => setImportVisible(true)}>
      <main className={styles.page}>
        <div className={styles.breadcrumb}>{t('pluginPlatform.workspace.title')} / {pageTitle}</div>
        <header className={styles.pageHeader}>
          <div className={styles.headerCopy}>
            <h1>{pageTitle}</h1>
            <p>{pageSubtitle}</p>
          </div>
          {desktopShell && (
            <Button type='primary' icon={<AddOne />} onClick={() => navigate('/plugins/new')}>
              {t('pluginPlatform.actions.create')}
            </Button>
          )}
        </header>

        <div className={styles.libraryToolbar}>
          <Input
            className={styles.librarySearch}
            value={query}
            prefix={<Search />}
            allowClear
            placeholder={t('pluginPlatform.library.search')}
            aria-label={t('pluginPlatform.library.search')}
            onChange={setQuery}
          />
          <div className={styles.toolbarActions}>
            <Select
              className={styles.sortSelect}
              value={sort}
              aria-label={t('pluginPlatform.library.sortLabel')}
              onChange={(value) => setSort(value as LibrarySort)}
              options={[
                { value: 'recent', label: t('pluginPlatform.library.sortRecent') },
                { value: 'name', label: t('pluginPlatform.library.sortName') },
              ]}
            />
            <Button
              className={styles.iconButton}
              icon={<Refresh />}
              loading={loading}
              aria-label={t('pluginPlatform.actions.refresh')}
              title={t('pluginPlatform.actions.refresh')}
              onClick={() => void refresh()}
            />
          </div>
        </div>

        {error && <Alert type='error' content={error} showIcon />}
        {!desktopShell && <Alert type='info' content={t('pluginPlatform.readOnly.body')} showIcon />}

        {loading ? (
          <div className={styles.emptyState}><Spin /><span>{t('pluginPlatform.library.loading')}</span></div>
        ) : entries.length ? (
          <section className={styles.libraryTable} aria-label={pageTitle}>
            <div className={styles.libraryTableHeader} aria-hidden='true'>
              <span>{t('pluginPlatform.library.columns.plugin')}</span>
              <span>{t('pluginPlatform.library.columns.type')}</span>
              <span>{t('pluginPlatform.library.columns.updated')}</span>
              <span>{t('pluginPlatform.library.columns.status')}</span>
              <span>{t('pluginPlatform.library.columns.actions')}</span>
            </div>
            <div className={styles.libraryRows}>
              {entries.map((entry) => entry.kind === 'draft' ? (
                <DraftRow
                  key={entry.key}
                  draft={entry.draft}
                  locale={i18n.language}
                  onOpen={() => {
                    if (desktopShell) navigate(`/plugins/create/${encodeURIComponent(entry.draft.draft_id)}`);
                  }}
                  readOnly={!desktopShell}
                />
              ) : (
                <PluginRow
                  key={entry.key}
                  plugin={entry.plugin}
                  locale={i18n.language}
                  pinned={pinned.has(entry.plugin.plugin_id)}
                  busy={busyId === entry.plugin.plugin_id}
                  readOnly={!desktopShell}
                  emphasizeIssue={view === 'attention'}
                  onOpen={() => navigate(`/plugins/run/${encodeURIComponent(entry.plugin.plugin_id)}`)}
                  onToggle={(enabled) => void toggleEnabled(entry.plugin, enabled)}
                  onTogglePinned={() => void togglePinned(entry.plugin)}
                />
              ))}
            </div>
          </section>
        ) : (
          <div className={styles.emptyState}>
            <PluginVisual draft={view === 'drafts'} />
            <h2>{query ? t('pluginPlatform.library.noResultsTitle') : t('pluginPlatform.library.emptyTitle')}</h2>
            <p>{query ? t('pluginPlatform.library.noResultsBody') : t('pluginPlatform.library.emptyBody')}</p>
            {query ? (
              <Button onClick={() => setQuery('')}>{t('pluginPlatform.library.clearSearch')}</Button>
            ) : desktopShell ? (
              <Button type='primary' icon={<AddOne />} onClick={() => navigate('/plugins/new')}>
                {t('pluginPlatform.actions.create')}
              </Button>
            ) : null}
          </div>
        )}

        {desktopShell && <PluginImportDialog
          visible={importVisible}
          onCancel={() => setImportVisible(false)}
          onInstalled={(detail) => {
            setImportVisible(false);
            notifyPluginLibraryChanged();
            navigate(`/plugins/run/${encodeURIComponent(detail.summary.plugin_id)}`);
          }}
        />}
      </main>
    </PluginWorkspace>
  );
}

interface DraftRowProps {
  draft: PluginDraftSummary;
  locale: string;
  readOnly: boolean;
  onOpen: () => void;
}

function DraftRow({ draft, locale, readOnly, onOpen }: DraftRowProps) {
  const { t } = useTranslation();
  const attention = draftNeedsAttention(draft);
  return (
    <article className={`${styles.libraryRow} ${attention ? styles.libraryRowAttention : ''}`}>
      <button type='button' className={styles.pluginIdentity} disabled={readOnly} onClick={onOpen}>
        <PluginVisual draft />
        <span className={styles.pluginCopy}>
          <span className={styles.pluginNameLine}>
            <strong>{draft.display_name || t('pluginPlatform.library.untitled')}</strong>
            <span className={styles.statusPill} data-tone={attention ? 'attention' : 'draft'}>
              {t(`pluginPlatform.creator.status.${draft.status}`)}
            </span>
          </span>
          <span className={styles.pluginDescription}>{draft.description || t('pluginPlatform.library.draftDescription')}</span>
          {attention && draft.error_code && (
            <span className={styles.inlineIssue}><Attention />{draft.error_code}</span>
          )}
        </span>
      </button>
      <span className={styles.kindBadge} data-tone='draft'>{t('pluginPlatform.library.draft')}</span>
      <span className={styles.updatedText}>{formatRelativeTime(draft.updated_at_ms, locale)}</span>
      <span className={styles.statusPill} data-tone={attention ? 'attention' : 'draft'}>
        {t(`pluginPlatform.creator.status.${draft.status}`)}
      </span>
      <Button
        type='text'
        className={styles.draftActionButton}
        icon={<PreviewOpen />}
        disabled={readOnly}
        aria-label={t('pluginPlatform.library.continue')}
        title={t('pluginPlatform.library.continue')}
        onClick={onOpen}
      >
        <span className={styles.draftActionLabel}>{t('pluginPlatform.library.continue')}</span>
      </Button>
    </article>
  );
}

interface PluginRowProps {
  plugin: PluginSummary;
  locale: string;
  pinned: boolean;
  busy: boolean;
  readOnly: boolean;
  emphasizeIssue: boolean;
  onOpen: () => void;
  onToggle: (enabled: boolean) => void;
  onTogglePinned: () => void;
}

function PluginRow({
  plugin,
  locale,
  pinned,
  busy,
  readOnly,
  emphasizeIssue,
  onOpen,
  onToggle,
  onTogglePinned,
}: PluginRowProps) {
  const { t } = useTranslation();
  const shape = pluginShape(plugin);
  const attention = pluginNeedsAttention(plugin);
  const trashed = plugin.trashed_at_ms !== undefined;
  const statusTone = trashed ? 'trash' : attention ? 'attention' : plugin.enabled ? 'enabled' : 'disabled';
  const statusText = trashed
    ? t('pluginPlatform.library.trashed')
    : attention
      ? t('pluginPlatform.workspace.views.attention')
      : plugin.enabled
        ? t('pluginPlatform.actions.enabled')
        : t('pluginPlatform.actions.disabled');

  const menu = (
    <Menu onClickMenuItem={(key) => {
      if (key === 'open') onOpen();
      if (key === 'pin') onTogglePinned();
    }}>
      <Menu.Item key='open'>
        <span className={styles.menuItem}><PreviewOpen />{t('pluginPlatform.actions.open')}</span>
      </Menu.Item>
      {!trashed && !readOnly && <Menu.Item key='pin'>
        <span className={styles.menuItem}><Pushpin />{pinned ? t('pluginPlatform.library.unpin') : t('pluginPlatform.library.pin')}</span>
      </Menu.Item>}
    </Menu>
  );

  return (
    <article className={`${styles.libraryRow} ${attention ? styles.libraryRowAttention : ''} ${attention && emphasizeIssue ? styles.attentionCard : ''}`}>
      <button type='button' className={styles.pluginIdentity} onClick={onOpen}>
        <PluginVisual shape={shape} />
        <span className={styles.pluginCopy}>
          <span className={styles.pluginNameLine}>
            <strong>{plugin.display_name}</strong>
            {pinned && <Pushpin className={styles.pinnedIcon} theme='filled' />}
          </span>
          <span className={styles.pluginDescription}>{plugin.description}</span>
          {attention && !emphasizeIssue && (
            <span className={styles.inlineIssue}><Attention />{plugin.last_error || t('pluginPlatform.service.failed')}</span>
          )}
        </span>
      </button>
      <span className={styles.kindBadge} data-tone={shape}>{t(`pluginPlatform.shape.${shape}`)}</span>
      <span className={styles.updatedText} title={new Date(plugin.updated_at_ms).toLocaleString(locale)}>
        {formatRelativeTime(plugin.updated_at_ms, locale)}
        <small>v{plugin.active.package_version}</small>
      </span>
      <span className={styles.rowStatusControl}>
        {attention && emphasizeIssue ? (
          <span className={styles.statusPill} data-tone='attention'>{t('pluginPlatform.workspace.views.attention')}</span>
        ) : !trashed && !readOnly ? (
          <Switch
            size='small'
            checked={plugin.enabled}
            loading={busy}
            aria-label={t('pluginPlatform.library.toggle', { name: plugin.display_name })}
            onChange={onToggle}
          />
        ) : (
          <span className={styles.statusPill} data-tone={statusTone}>{statusText}</span>
        )}
      </span>
      <Dropdown droplist={menu} trigger='click' position='br' getPopupContainer={() => document.body}>
        <Button
          type='text'
          className={styles.moreButton}
          icon={<MoreOne />}
          aria-label={t('pluginPlatform.library.moreActions', { name: plugin.display_name })}
        />
      </Dropdown>
      {attention && emphasizeIssue && (
        <div className={styles.attentionResolution}>
          <span className={styles.attentionResolutionIcon}><Attention /></span>
          <span className={styles.attentionResolutionCopy}>
            <strong>{t('pluginPlatform.library.issueTitle')}</strong>
            <small>{plugin.last_error || t('pluginPlatform.service.failed')}</small>
          </span>
          <Button type='primary' icon={<PreviewOpen />} onClick={onOpen}>
            {t('pluginPlatform.library.reviewIssue')}
          </Button>
        </div>
      )}
    </article>
  );
}

function formatRelativeTime(timestamp: number, locale: string): string {
  if (!Number.isFinite(timestamp)) return '—';
  const seconds = Math.round((timestamp - Date.now()) / 1000);
  const formatter = new Intl.RelativeTimeFormat(locale, { numeric: 'auto' });
  if (Math.abs(seconds) < 60) return formatter.format(seconds, 'second');
  const minutes = Math.round(seconds / 60);
  if (Math.abs(minutes) < 60) return formatter.format(minutes, 'minute');
  const hours = Math.round(minutes / 60);
  if (Math.abs(hours) < 24) return formatter.format(hours, 'hour');
  const days = Math.round(hours / 24);
  if (Math.abs(days) < 30) return formatter.format(days, 'day');
  return new Intl.DateTimeFormat(locale, { month: 'short', day: 'numeric' }).format(timestamp);
}
