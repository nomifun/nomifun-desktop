import { useEffect, useRef, type ReactNode } from 'react';
import { AllApplication, EditTwo, Delete, ApplicationMenu, ApiApp, Puzzle } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import { useLocation, useNavigate } from 'react-router-dom';
import type { PluginLibraryCounts, PluginLibraryView, PluginShape } from './pluginPlatformModel';
import styles from './PluginPlatform.module.css';

interface PluginWorkspaceProps {
  children: ReactNode;
  activeView?: PluginLibraryView;
  counts?: PluginLibraryCounts;
}

export default function PluginWorkspace({ children, activeView = 'all', counts }: PluginWorkspaceProps) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const location = useLocation();
  const mainRef = useRef<HTMLDivElement>(null);
  const current = activeView === 'drafts' || location.pathname.includes('/authoring') || location.pathname === '/plugins/create'
    ? 'drafts' : activeView === 'trash' ? 'trash' : 'all';
  useEffect(() => { mainRef.current?.scrollTo({ top: 0 }); }, [location.pathname, location.search]);
  return <div className={styles.workspace}>
    <header className={styles.workspaceHeader}>
      <button type='button' className={styles.workspaceBrand} onClick={() => navigate('/plugins')}>
        <span className={styles.workspaceMark}><Puzzle /></span>
        <strong>{t('pluginPlatform.workspace.title')}</strong>
      </button>
      <nav className={styles.workspaceTabs} aria-label={t('pluginPlatform.workspace.navigation')}>
        {([
          { key: 'all', icon: <AllApplication /> },
          { key: 'drafts', icon: <EditTwo /> },
          { key: 'trash', icon: <Delete /> },
        ] as const).map(({ key, icon }) => <button key={key} type='button'
          className={current === key ? styles.workspaceTabActive : ''}
          aria-current={current === key ? 'page' : undefined}
          onClick={() => navigate(key === 'all' ? '/plugins' : '/plugins?view=' + key)}>
          {icon}<span>{t('pluginPlatform.workspace.views.' + key)}</span>
          {counts && <small>{counts[key]}</small>}
        </button>)}
      </nav>
    </header>
    <div ref={mainRef} className={styles.workspaceMain}>{children}</div>
  </div>;
}

export function PluginVisual({ shape = 'mixed', draft = false, large = false }: { shape?: PluginShape; draft?: boolean; large?: boolean }) {
  return <span className={styles.pluginVisual + (large ? ' ' + styles.pluginVisualLarge : '')}
    data-tone={draft ? 'draft' : shape} aria-hidden='true'>
    {draft ? <EditTwo /> : shape === 'ui_only' ? <ApplicationMenu /> : shape === 'headless' ? <ApiApp /> : <Puzzle />}
  </span>;
}
