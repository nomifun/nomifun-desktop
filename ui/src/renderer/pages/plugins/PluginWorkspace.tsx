import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import {
  AddOne,
  AllApplication,
  ApiApp,
  ApplicationMenu,
  Attention,
  Delete,
  EditTwo,
  Left,
  PauseOne,
  PlayOne,
  Puzzle,
  Right,
  Shield,
  Upload,
} from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import { useLocation, useNavigate } from 'react-router-dom';
import { pluginPlatform } from '@/common/adapter/pluginPlatformBridge';
import { isDesktopShell } from '@/renderer/utils/platform';
import { PLUGIN_LIBRARY_CHANGED } from './pluginLibraryState';
import {
  pluginLibraryCounts,
  type PluginLibraryCounts,
  type PluginLibraryView,
  type PluginShape,
} from './pluginPlatformModel';
import styles from './PluginPlatform.module.css';

const emptyCounts: PluginLibraryCounts = {
  all: 0,
  enabled: 0,
  disabled: 0,
  drafts: 0,
  attention: 0,
  trash: 0,
  ui_only: 0,
  headless: 0,
  mixed: 0,
};

interface PluginWorkspaceProps {
  children: ReactNode;
  activeView?: PluginLibraryView;
  counts?: PluginLibraryCounts;
  onImport?: () => void;
}

export default function PluginWorkspace({
  children,
  activeView = 'all',
  counts: providedCounts,
  onImport,
}: PluginWorkspaceProps) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const location = useLocation();
  const desktopShell = isDesktopShell();
  const [collapsed, setCollapsed] = useState(false);
  const [loadedCounts, setLoadedCounts] = useState<PluginLibraryCounts>(emptyCounts);
  const mainRef = useRef<HTMLDivElement>(null);

  const refreshCounts = useCallback(async () => {
    if (providedCounts) return;
    try {
      const [library, draftList] = await Promise.all([
        pluginPlatform.plugins.list.invoke(),
        pluginPlatform.drafts.list.invoke(),
      ]);
      setLoadedCounts(pluginLibraryCounts(library.plugins, draftList.drafts));
    } catch {
      // Navigation remains useful while the backend reconnects; keep the last counts.
    }
  }, [providedCounts]);

  useEffect(() => {
    void refreshCounts();
    window.addEventListener(PLUGIN_LIBRARY_CHANGED, refreshCounts);
    return () => window.removeEventListener(PLUGIN_LIBRARY_CHANGED, refreshCounts);
  }, [refreshCounts]);

  useEffect(() => {
    mainRef.current?.scrollTo({ top: 0, left: 0 });
  }, [location.pathname, location.search]);

  const counts = providedCounts ?? loadedCounts;
  const smartViews = useMemo(() => ([
    { key: 'all' as const, icon: <AllApplication />, label: t('pluginPlatform.workspace.views.all') },
    { key: 'enabled' as const, icon: <PlayOne />, label: t('pluginPlatform.workspace.views.enabled') },
    { key: 'disabled' as const, icon: <PauseOne />, label: t('pluginPlatform.workspace.views.disabled') },
    { key: 'drafts' as const, icon: <EditTwo />, label: t('pluginPlatform.workspace.views.drafts') },
    { key: 'attention' as const, icon: <Attention />, label: t('pluginPlatform.workspace.views.attention') },
    { key: 'trash' as const, icon: <Delete />, label: t('pluginPlatform.workspace.views.trash') },
  ]), [t]);
  const shapes = useMemo(() => ([
    { key: 'ui_only' as const, icon: <ApplicationMenu />, label: t('pluginPlatform.workspace.views.ui_only') },
    { key: 'headless' as const, icon: <ApiApp />, label: t('pluginPlatform.workspace.views.headless') },
    { key: 'mixed' as const, icon: <Puzzle />, label: t('pluginPlatform.workspace.views.mixed') },
  ]), [t]);

  const openView = (view: PluginLibraryView) => {
    navigate(view === 'all' ? '/plugins' : `/plugins?view=${view}`);
  };

  return (
    <div className={styles.workspace}>
      <div className={`${styles.workspaceFrame} ${collapsed ? styles.workspaceFrameCollapsed : ''}`}>
        <aside className={styles.workspaceNav} aria-label={t('pluginPlatform.workspace.navigation')}>
          <div className={styles.workspaceNavHeader}>
            <div className={styles.workspaceTitleBlock}>
              <div className={styles.workspaceMark}><Puzzle /></div>
              <div className={styles.workspaceTitleText}>
                <strong>{t('pluginPlatform.workspace.title')}</strong>
                <span>{t('pluginPlatform.workspace.subtitle')}</span>
              </div>
            </div>
            <button
              type='button'
              className={styles.collapseButton}
              aria-label={collapsed ? t('pluginPlatform.workspace.expand') : t('pluginPlatform.workspace.collapse')}
              title={collapsed ? t('pluginPlatform.workspace.expand') : t('pluginPlatform.workspace.collapse')}
              onClick={() => setCollapsed((value) => !value)}
            >
              {collapsed ? <Right /> : <Left />}
            </button>
          </div>

          {desktopShell && (
            <div className={styles.workspacePrimaryActions}>
              <button
                type='button'
                className={styles.createButton}
                title={t('pluginPlatform.actions.create')}
                onClick={() => navigate('/plugins/new')}
              >
                <AddOne /><span className={styles.workspaceNavText}>{t('pluginPlatform.actions.create')}</span>
              </button>
              <button
                type='button'
                className={styles.importButton}
                title={t('pluginPlatform.actions.import')}
                onClick={() => onImport ? onImport() : navigate('/plugins?import=1')}
              >
                <Upload /><span className={styles.workspaceNavText}>{t('pluginPlatform.actions.import')}</span>
              </button>
            </div>
          )}

          <nav className={styles.workspaceNavBody}>
            <PluginNavGroup
              title={t('pluginPlatform.workspace.smartViews')}
              items={smartViews}
              counts={counts}
              activeView={activeView}
              onSelect={openView}
            />
            <PluginNavGroup
              title={t('pluginPlatform.workspace.capabilityTypes')}
              items={shapes}
              counts={counts}
              activeView={activeView}
              onSelect={openView}
            />
          </nav>

          <div className={styles.workspaceNavFoot} title={t('pluginPlatform.workspace.localOnly')}>
            <Shield />
            <span className={styles.workspaceNavText}>{t('pluginPlatform.workspace.localOnly')}</span>
          </div>
        </aside>
        <div
          key={`${location.key}:${location.pathname}:${location.search}`}
          ref={mainRef}
          className={styles.workspaceMain}
        >
          {children}
        </div>
      </div>
    </div>
  );
}

interface PluginNavGroupProps {
  title: string;
  items: ReadonlyArray<{ key: PluginLibraryView; icon: ReactNode; label: string }>;
  counts: PluginLibraryCounts;
  activeView: PluginLibraryView;
  onSelect: (view: PluginLibraryView) => void;
}

function PluginNavGroup({ title, items, counts, activeView, onSelect }: PluginNavGroupProps) {
  return (
    <section className={styles.workspaceNavGroup}>
      <h2 className={styles.workspaceNavGroupTitle}>{title}</h2>
      <div className={styles.workspaceNavItems}>
        {items.map((item) => (
          <button
            key={item.key}
            type='button'
            className={`${styles.workspaceNavItem} ${activeView === item.key ? styles.workspaceNavItemActive : ''}`}
            aria-current={activeView === item.key ? 'page' : undefined}
            title={item.label}
            onClick={() => onSelect(item.key)}
          >
            <span className={styles.workspaceNavIcon}>{item.icon}</span>
            <span className={`${styles.workspaceNavText} ${styles.workspaceNavLabel}`}>{item.label}</span>
            <span className={`${styles.workspaceNavText} ${styles.workspaceNavCount}`}>{counts[item.key]}</span>
          </button>
        ))}
      </div>
    </section>
  );
}

interface PluginVisualProps {
  shape?: PluginShape;
  draft?: boolean;
  large?: boolean;
}

export function PluginVisual({ shape = 'mixed', draft = false, large = false }: PluginVisualProps) {
  const icon = draft
    ? <EditTwo />
    : shape === 'ui_only'
      ? <ApplicationMenu />
      : shape === 'headless'
        ? <ApiApp />
        : <Puzzle />;
  return (
    <span
      className={`${styles.pluginVisual} ${large ? styles.pluginVisualLarge : ''}`}
      data-tone={draft ? 'draft' : shape}
      aria-hidden='true'
    >
      {icon}
    </span>
  );
}
