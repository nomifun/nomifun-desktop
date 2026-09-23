import { useCallback, useEffect, useMemo, useState } from 'react';
import { Alert, Button, Input, Spin, Switch, Tag } from '@arco-design/web-react';
import { AddOne, Plug, Pushpin, Search, Upload } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import { pluginPlatform } from '@/common/adapter/pluginPlatformBridge';
import type {
  PluginDraftSummary,
  PluginLibraryState,
  PluginSummary,
} from '@/common/types/pluginPlatform';
import { isDesktopShell } from '@/renderer/utils/platform';
import PluginImportDialog from './PluginImportDialog';
import {
  PLUGIN_LIBRARY_CHANGED,
  notifyPluginLibraryChanged,
  updatePluginLibraryState,
} from './pluginLibraryState';
import {
  pluginLibraryEntries,
  pluginShape,
} from './pluginPlatformModel';
import styles from './PluginPlatform.module.css';

export default function PluginLibraryPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [plugins, setPlugins] = useState<PluginSummary[]>([]);
  const [drafts, setDrafts] = useState<PluginDraftSummary[]>([]);
  const [organization, setOrganization] = useState<PluginLibraryState>({
    revision: 0,
    collections: [],
    items: [],
  });
  const [query, setQuery] = useState('');
  const [loading, setLoading] = useState(true);
  const [busyId, setBusyId] = useState('');
  const [error, setError] = useState('');
  const [importVisible, setImportVisible] = useState(false);
  const desktopShell = isDesktopShell();

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

  const entries = useMemo(() => {
    const normalized = query.trim().toLocaleLowerCase();
    return pluginLibraryEntries(plugins, drafts).filter((entry) => {
      if (!normalized) return true;
      const text = entry.kind === 'plugin'
        ? `${entry.plugin.display_name} ${entry.plugin.description} ${entry.plugin.package_id}`
        : `${entry.draft.display_name} ${entry.draft.description} ${entry.draft.package_id ?? ''}`;
      return text.toLocaleLowerCase().includes(normalized);
    });
  }, [drafts, plugins, query]);

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

  return (
    <main className={styles.page}>
      <header className={styles.header}>
        <div className={styles.headerCopy}>
          <h1>{t('pluginPlatform.library.title')}</h1>
          <p>{t('pluginPlatform.library.subtitle')}</p>
        </div>
        {desktopShell && <div className={styles.actions}>
          <Button icon={<Upload />} onClick={() => setImportVisible(true)}>
            {t('pluginPlatform.actions.import')}
          </Button>
          <Button type='primary' icon={<AddOne />} onClick={() => navigate('/plugins/new')}>
            {t('pluginPlatform.actions.create')}
          </Button>
        </div>}
      </header>
      <div className={styles.toolbar}>
        <Input
          className={styles.toolbarSearch}
          value={query}
          prefix={<Search />}
          allowClear
          placeholder={t('pluginPlatform.library.search')}
          onChange={setQuery}
        />
        <Button onClick={() => void refresh()}>{t('pluginPlatform.actions.refresh')}</Button>
      </div>
      {error && <Alert type='error' content={error} />}
      {!desktopShell && (
        <Alert type='info' content={t('pluginPlatform.readOnly.body')} />
      )}
      {loading ? (
        <div className={styles.empty}><Spin /><span>{t('pluginPlatform.library.loading')}</span></div>
      ) : entries.length ? (
        <div className={styles.grid}>
          {entries.map((entry) => {
            if (entry.kind === 'draft') {
              return (
                <article key={entry.key} className={styles.card}>
                  <div className={styles.cardHeader}>
                    <Tag color='arcoblue'>{t('pluginPlatform.library.draft')}</Tag>
                    <span className={styles.muted}>{entry.draft.status}</span>
                  </div>
                  <button
                    type='button'
                    className={styles.cardButton}
                    disabled={!desktopShell}
                    onClick={() => {
                      if (desktopShell) navigate(`/plugins/create/${encodeURIComponent(entry.draft.draft_id)}`);
                    }}
                  >
                    <strong>{entry.draft.display_name || t('pluginPlatform.library.untitled')}</strong>
                    <span>{entry.draft.package_id ?? t('pluginPlatform.library.unsaved')}</span>
                  </button>
                  <p className={styles.cardDescription}>{entry.draft.description}</p>
                  <div className={styles.cardFooter}>
                    <span>{desktopShell
                      ? t('pluginPlatform.library.continue')
                      : t('pluginPlatform.readOnly.draft')}</span>
                    {desktopShell && <Button size='small' onClick={() => navigate(`/plugins/create/${encodeURIComponent(entry.draft.draft_id)}`)}>
                      {t('pluginPlatform.actions.open')}
                    </Button>}
                  </div>
                </article>
              );
            }
            const plugin = entry.plugin;
            const trashed = plugin.trashed_at_ms !== undefined;
            return (
              <article key={entry.key} className={styles.card}>
                <div className={styles.cardHeader}>
                  <Tag>{t(`pluginPlatform.shape.${pluginShape(plugin)}`)}</Tag>
                  {!trashed && desktopShell && (
                    <Switch
                      size='small'
                      checked={plugin.enabled}
                      loading={busyId === plugin.plugin_id}
                      aria-label={t('pluginPlatform.library.toggle', { name: plugin.display_name })}
                      onChange={(enabled) => void toggleEnabled(plugin, enabled)}
                    />
                  )}
                </div>
                <button
                  type='button'
                  className={styles.cardButton}
                  onClick={() => navigate(`/plugins/run/${encodeURIComponent(plugin.plugin_id)}`)}
                >
                  <strong>{plugin.display_name}</strong>
                  <span>{plugin.package_id} · {plugin.active.package_version}</span>
                </button>
                <p className={styles.cardDescription}>{plugin.description}</p>
                <div className={styles.cardFooter}>
                  {desktopShell && <Button
                    type='text'
                    size='small'
                    icon={<Pushpin theme={pinned.has(plugin.plugin_id) ? 'filled' : 'outline'} />}
                    aria-pressed={pinned.has(plugin.plugin_id)}
                    onClick={() => void togglePinned(plugin)}
                  />}
                  <span>{trashed
                    ? t('pluginPlatform.library.trashed')
                    : plugin.has_ui && plugin.enabled
                      ? t('pluginPlatform.library.openApp')
                      : t(`pluginPlatform.service.${plugin.runtime.state}`)}</span>
                  <Button size='small' icon={<Plug />} onClick={() => navigate(`/plugins/run/${encodeURIComponent(plugin.plugin_id)}`)}>
                    {t('pluginPlatform.actions.open')}
                  </Button>
                </div>
              </article>
            );
          })}
        </div>
      ) : (
        <div className={styles.empty}>
          <Plug size={28} />
          <h2>{t('pluginPlatform.library.emptyTitle')}</h2>
          <p>{t('pluginPlatform.library.emptyBody')}</p>
          {desktopShell && <Button type='primary' onClick={() => navigate('/plugins/new')}>
            {t('pluginPlatform.actions.create')}
          </Button>}
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
  );
}
