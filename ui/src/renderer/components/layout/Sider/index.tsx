/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { Suspense, useCallback, useEffect, useRef } from 'react';
import PluginPinnedEntries from '@/renderer/pages/plugins/PluginPinnedEntries';
import { useTranslation } from 'react-i18next';
import { useLocation, useNavigate } from 'react-router-dom';
import { preloadResourceRoute } from '@renderer/components/layout/Router';
import { cleanupSiderTooltips, getSiderTooltipProps } from '@renderer/utils/ui/siderTooltip';
import { useAuth } from '@renderer/hooks/context/AuthContext';
import { blurActiveElement } from '@renderer/utils/ui/focus';
import { isDesktopShell } from '@renderer/utils/platform';
import { useBrowserOverview } from '@renderer/pages/browser/useBrowserInventory';
import { parseSessionRoute } from '@renderer/utils/routes/sessionRoute';
import { CANVASES_PATH, MATERIALS_PATH, PROMPTS_PATH, TEMPLATES_PATH } from '@renderer/pages/creativeStudio/app/resourceRoutes';
import { FileText, FullScreen, PageTemplate } from '@icon-park/react';
import SiderResourceEntry from './SiderNav/SiderResourceEntry';
import { requestCreativeStudioBeforeLeave } from '@renderer/pages/creativeStudio/app/beforeLeave';
import { readCanvasResumeLocation, rememberCanvasResumeLocation } from '@renderer/pages/creativeStudio/app/canvasResumeLocation';
import {
  SiderAssetLibraryEntry,
  SiderAgentEntry,
  SiderBrowserEntry,
  SiderSkillsEntry,
  SiderConversationEntry,
  SiderCustomerServiceEntry,
  SiderKnowledgeEntry,
  SiderMcpEntry,
  SiderModelHubEntry,
  SiderNomiEntry,
  SiderOpenCapabilitiesEntry,
  SiderPluginEntry,
  SiderRequirementsEntry,
  SiderScheduledEntry,
  SiderSectionHeader,
} from './SiderNav';
import SiderFooter from './SiderFooter';

const SettingsSider = React.lazy(() => import('@renderer/pages/settings/components/SettingsSider'));

interface SiderProps {
  onSessionClick?: () => void;
  collapsed?: boolean;
}

/**
 * Sider — the app-level primary navigation rail.
 *
 * Slimmed down to a pure capability rail: the conversation/terminal session
 * list, the create switches, and full-text search were lifted out into the
 * content-area secondary sidebar (`ConversationShell` / `ContentSider`),
 * reached via the "会话" entry. The rail holds top-level destinations grouped
 * by small-text section headers (`SiderSectionHeader`): 常用 (会话 / 桌面伙伴),
 * 数据空间 (知识库), 自动化 (定时任务 / 需求平台),
 * 增强工具 (设定 / Skill / MCP), 服务 (客服), and a bottom-pinned 设置 group
 * (浏览器管理 + 模型管理 + the footer). Execution engines live as an
 * independent tab inside Settings rather than being mixed into model
 * management.
 */
const Sider: React.FC<SiderProps> = ({ onSessionClick, collapsed = false }) => {
  const { t } = useTranslation();
  const location = useLocation();
  const { pathname, search, hash } = location;
  const {
    overview: browserOverview,
    transient: browserOverviewTransient,
    retry: retryBrowserOverview,
  } = useBrowserOverview();

  const navigate = useNavigate();
  const { logout, status } = useAuth();
  const isSettings = pathname.startsWith('/settings');
  const currentPath = `${pathname}${search}${hash}`;
  const lastNonSettingsPathRef = useRef('/guid');
  const lastCanvasPathRef = useRef(readCanvasResumeLocation());
  const pendingNavigationRef = useRef<{ target: string; replace: boolean; state?: unknown } | null>(null);
  const navigationInFlightRef = useRef(false);
  // Logout is a WebUI-only affordance: the bundled desktop shell (Electron or
  // Tauri) is single-user with no auth, so there is nothing to log out of.
  const showLogout = !isDesktopShell() && status === 'authenticated';

  useEffect(() => {
    if (!isSettings) {
      lastNonSettingsPathRef.current = currentPath;
    }
  }, [currentPath, isSettings]);

  useEffect(() => {
    const remembered = rememberCanvasResumeLocation(currentPath);
    if (remembered) lastCanvasPathRef.current = remembered;
  }, [currentPath]);

  const navTo = useCallback(
    (target: string, replace = false, state?: unknown) => {
      cleanupSiderTooltips();
      blurActiveElement();
      pendingNavigationRef.current = { target, replace, state };
      if (navigationInFlightRef.current) return;
      navigationInFlightRef.current = true;
      void (async () => {
        let allowed = false;
        try { allowed = await requestCreativeStudioBeforeLeave(); } catch { /* A failed canvas save keeps its page open. */ }
        const pending = pendingNavigationRef.current;
        pendingNavigationRef.current = null;
        navigationInFlightRef.current = false;
        if (!allowed || !pending) return;
        try {
          await navigate(pending.target, { replace: pending.replace, state: pending.state });
          onSessionClick?.();
        } catch (error) { console.error('Navigation failed:', error); }
      })();
    },
    [navigate, onSessionClick]
  );

  const handleConversationClick = () =>
    navTo('/guid');
  const handleBrowserClick = () => {
    if (browserOverviewTransient) {
      void retryBrowserOverview();
    }
    const currentSession = parseSessionRoute(pathname);
    if (currentSession?.kind === 'conversation') {
      navTo(`/browser?conversation_id=${encodeURIComponent(currentSession.id)}`);
      return;
    }
    navTo(pathname === '/browser' && search ? `/browser${search}` : '/browser');
  };
  const handleScheduledClick = () => navTo('/scheduled');
  const handleRequirementsClick = () => navTo('/requirements');
  const handleKnowledgeClick = () => navTo('/knowledge');
  const handleAssetLibraryClick = () => {
    void preloadResourceRoute(MATERIALS_PATH);
    navTo(MATERIALS_PATH);
  };
  const handleNomiClick = () => navTo('/nomi');
  const handleCanvasClick = () => {
    void preloadResourceRoute(lastCanvasPathRef.current);
    navTo(lastCanvasPathRef.current);
  };
  const handleCustomerServiceClick = () => navTo('/customer-service');
  const handleAgentClick = () => navTo('/agent');
  const handleSkillsClick = () => navTo('/skills');
  const handlePluginClick = () => navTo('/plugins');
  const handleMcpClick = () => navTo('/mcp');
  const handleOpenCapabilitiesClick = () => navTo('/open-capabilities');
  const handleModelHubClick = () => navTo('/models');
  const handleSettingsClick = () => navTo(isSettings ? lastNonSettingsPathRef.current || '/guid' : '/settings/system');

  const handleLogout = useCallback(async () => {
    cleanupSiderTooltips();
    blurActiveElement();
    try {
      await logout();
    } catch (error) {
      console.error('Logout failed:', error);
      return; // logout 失败时不执行后续操作
    }
    if (onSessionClick) {
      onSessionClick();
    }
  }, [logout, onSessionClick]);

  useEffect(() => {
    if (!showLogout) return;

    const handleKeyDown = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.shiftKey && event.key.toLowerCase() === 'l') {
        event.preventDefault();
        handleLogout();
      }
    };

    window.addEventListener('keydown', handleKeyDown);
    return () => {
      window.removeEventListener('keydown', handleKeyDown);
    };
  }, [handleLogout, showLogout]);

  const tooltipEnabled = collapsed;
  const siderTooltipProps = getSiderTooltipProps(tooltipEnabled);

  // The "会话" entry stays active across every route owned by ConversationShell.
  const isSessionRoute =
    pathname === '/guid' ||
    pathname.startsWith('/conversation/') ||
    pathname === '/terminal-new' ||
    pathname.startsWith('/terminal/');

  return (
    <div className='size-full flex flex-col'>
      {/* Main content area */}
      <div className='flex-1 min-h-0 overflow-y-auto overflow-x-hidden'>
        {isSettings ? (
          <Suspense fallback={<div className='size-full' />}>
            <SettingsSider collapsed={collapsed} tooltipEnabled={tooltipEnabled} />
          </Suspense>
        ) : (
          <div className='size-full flex flex-col gap-1px'>
            {/* 常用 — high-frequency primary destinations */}
            <SiderSectionHeader label={t('common.siderSection.common')} collapsed={collapsed} />
            {/* Conversations — opens the session secondary sidebar (ContentSider) */}
            <SiderConversationEntry
              isActive={isSessionRoute}
              collapsed={collapsed}
              siderTooltipProps={siderTooltipProps}
              onClick={handleConversationClick}
            />
            {/* Agent authoring workbench */}
            <SiderAgentEntry
              isActive={pathname === '/agent' || pathname.startsWith('/agent-sessions/')}
              collapsed={collapsed}
              siderTooltipProps={siderTooltipProps}
              onClick={handleAgentClick}
            />
            {/* Work partner (桌面伙伴) */}
            <SiderNomiEntry
              isActive={pathname === '/nomi'}
              collapsed={collapsed}
              siderTooltipProps={siderTooltipProps}
              onClick={handleNomiClick}
            />
            <SiderResourceEntry
              label={t('creativeStudio.navigation.canvases', { defaultValue: '我的画布' })}
              icon={<FullScreen theme='outline' size={collapsed ? 20 : 16} fill='currentColor' />}
              isActive={pathname === CANVASES_PATH || pathname.startsWith(CANVASES_PATH + '/')}
              collapsed={collapsed} siderTooltipProps={siderTooltipProps} onClick={handleCanvasClick}
            />
            {/* 数据空间 — data & storage (文件管理 reserved for later) */}
            <SiderSectionHeader label={t('common.siderSection.data')} collapsed={collapsed} />
            {/* Knowledge base */}
            <SiderKnowledgeEntry
              isActive={pathname.startsWith('/knowledge')}
              collapsed={collapsed}
              siderTooltipProps={siderTooltipProps}
              onClick={handleKnowledgeClick}
            />
            {/* Asset library — retained materials page. */}
            <div
              className='shrink-0'
              onMouseEnter={() => void preloadResourceRoute(MATERIALS_PATH)}
              onFocusCapture={() => void preloadResourceRoute(MATERIALS_PATH)}
            >
              <SiderAssetLibraryEntry
                isActive={pathname === MATERIALS_PATH}
                collapsed={collapsed}
                siderTooltipProps={siderTooltipProps}
                onClick={handleAssetLibraryClick}
              />
            </div>
            <SiderResourceEntry
              label={t('creativeStudio.navigation.prompts', { defaultValue: '提示词库' })}
              icon={<FileText theme='outline' size={collapsed ? 20 : 16} fill='currentColor' />}
              isActive={pathname === PROMPTS_PATH} collapsed={collapsed} siderTooltipProps={siderTooltipProps}
              onClick={() => navTo(PROMPTS_PATH)}
            />
            <SiderResourceEntry
              label={t('creativeStudio.navigation.templates', { defaultValue: '模板工作台' })}
              icon={<PageTemplate theme='outline' size={collapsed ? 20 : 16} fill='currentColor' />}
              isActive={pathname === TEMPLATES_PATH} collapsed={collapsed} siderTooltipProps={siderTooltipProps}
              onClick={() => navTo(TEMPLATES_PATH)}
            />
            {/* 自动化 — automation platforms */}
            <SiderSectionHeader label={t('common.siderSection.automation')} collapsed={collapsed} />
            {/* Scheduled tasks */}
            <SiderScheduledEntry
              isActive={pathname === '/scheduled'}
              collapsed={collapsed}
              siderTooltipProps={siderTooltipProps}
              onClick={handleScheduledClick}
            />
            {/* Requirements platform */}
            <SiderRequirementsEntry
              isActive={pathname.startsWith('/requirements')}
              collapsed={collapsed}
              siderTooltipProps={siderTooltipProps}
              onClick={handleRequirementsClick}
            />
            {/* 增强工具 — extension capabilities */}
            <SiderSectionHeader label={t('common.siderSection.tools')} collapsed={collapsed} />
            {/* Skills and MCP remain platform capability destinations. */}
            <SiderSkillsEntry
              isActive={pathname.startsWith('/skills')}
              collapsed={collapsed}
              siderTooltipProps={siderTooltipProps}
              onClick={handleSkillsClick}
            />
            <SiderPluginEntry
                isActive={pathname.startsWith('/plugins')}
                collapsed={collapsed}
                siderTooltipProps={siderTooltipProps}
                onClick={handlePluginClick}
              />
            <PluginPinnedEntries collapsed={collapsed} />
            {/* MCP — MCP tool server configuration */}
            <SiderMcpEntry
              isActive={pathname.startsWith('/mcp')}
              collapsed={collapsed}
              siderTooltipProps={siderTooltipProps}
              onClick={handleMcpClick}
            />
            {/* 服务 — public-facing services (客服), a domain fully separate
                from the desktop-companion group above. */}
            <SiderSectionHeader label={t('common.siderSection.services')} collapsed={collapsed} />
            <SiderCustomerServiceEntry
              isActive={pathname.startsWith('/customer-service')}
              collapsed={collapsed}
              siderTooltipProps={siderTooltipProps}
              onClick={handleCustomerServiceClick}
            />
          </div>
        )}
      </div>
      {/* Bottom pinned settings group. */}
      <div className='shrink-0 mt-auto pt-5px flex flex-col gap-1px border-t border-solid border-[var(--color-border-2)] border-l-0 border-r-0 border-b-0'>
        <>
            {/* 设置 — section label; the enclosing border-t already separates this region when collapsed */}
            <SiderSectionHeader label={t('common.siderSection.settings')} collapsed={collapsed} collapsedRule={false} />
            {/* Unified Browser management — keep the entry reachable when Browser Use is
                disabled so the user can open Settings and turn it back on. */}
            {(isDesktopShell() || browserOverview?.supported !== false) && (
              <SiderBrowserEntry
                isActive={pathname === '/browser'}
                collapsed={collapsed}
                runningCount={browserOverview?.running_lanes ?? 0}
                queuedCount={browserOverview?.queued_lanes ?? 0}
                siderTooltipProps={siderTooltipProps}
                onClick={handleBrowserClick}
              />
            )}
            <SiderModelHubEntry
              isActive={pathname.startsWith('/models')}
              collapsed={collapsed}
              siderTooltipProps={siderTooltipProps}
              onClick={handleModelHubClick}
            />
            <SiderOpenCapabilitiesEntry
              isActive={pathname.startsWith('/open-capabilities')}
              collapsed={collapsed}
              siderTooltipProps={siderTooltipProps}
              onClick={handleOpenCapabilitiesClick}
            />
            <SiderFooter
              isSettings={isSettings}
              collapsed={collapsed}
              siderTooltipProps={siderTooltipProps}
              onSettingsClick={handleSettingsClick}
              showLogout={showLogout}
              onLogoutClick={handleLogout}
            />
        </>
      </div>
    </div>
  );
};

export default Sider;
