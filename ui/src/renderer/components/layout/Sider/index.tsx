/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { Suspense, useCallback, useEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { useLocation, useNavigate } from 'react-router-dom';
import { cleanupSiderTooltips, getSiderTooltipProps } from '@renderer/utils/ui/siderTooltip';
import { useAuth } from '@renderer/hooks/context/AuthContext';
import { useLayoutContext } from '@renderer/hooks/context/LayoutContext';
import { blurActiveElement } from '@renderer/utils/ui/focus';
import { isDesktopShell } from '@renderer/utils/platform';
import { useBrowserOverview } from '@renderer/pages/browser/useBrowserInventory';
import { parseSessionRoute } from '@renderer/utils/routes/sessionRoute';
import {
  CREATIVE_STUDIO_ASSETS_PATH,
  CREATIVE_STUDIO_PROJECTS_PATH,
  WORKBENCH_HOME_PATH,
  isCreativeStudioPath,
} from '@renderer/pages/creativeStudio/app/routes';
import { requestCreativeStudioBeforeLeave } from '@renderer/pages/creativeStudio/app/beforeLeave';
import {
  SiderAssetLibraryEntry,
  SiderBrowserEntry,
  SiderCreativeStudioEntry,
  SiderPresetEntry,
  SiderSkillsEntry,
  SiderConversationEntry,
  SiderCustomerServiceEntry,
  SiderKnowledgeEntry,
  SiderMcpEntry,
  SiderMiniAppsEntry,
  SiderModelHubEntry,
  SiderNomiEntry,
  SiderOpenCapabilitiesEntry,
  SiderRequirementsEntry,
  SiderScheduledEntry,
  SiderSectionHeader,
} from './SiderNav';
import SiderFooter from './SiderFooter';

const SettingsSider = React.lazy(() => import('@renderer/pages/settings/components/SettingsSider'));
const CreativeStudioSider = React.lazy(
  () => import('@renderer/pages/creativeStudio/app/CreativeStudioSider')
);

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
  const layout = useLayoutContext();
  const isMobile = layout?.isMobile ?? false;
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
  const isCreativeStudio = isCreativeStudioPath(pathname);
  const lastNonSettingsPathRef = useRef('/guid');
  // Logout is a WebUI-only affordance: the bundled desktop shell (Electron or
  // Tauri) is single-user with no auth, so there is nothing to log out of.
  const showLogout = !isDesktopShell() && status === 'authenticated';

  useEffect(() => {
    if (!pathname.startsWith('/settings')) {
      lastNonSettingsPathRef.current = `${pathname}${search}${hash}`;
    }
  }, [pathname, search, hash]);

  const navTo = useCallback(
    (target: string, replace = false) => {
      cleanupSiderTooltips();
      blurActiveElement();
      Promise.resolve(navigate(target, { replace })).catch((error) => {
        console.error('Navigation failed:', error);
      });
      if (onSessionClick) {
        onSessionClick();
      }
    },
    [navigate, onSessionClick]
  );

  const handleConversationClick = () => navTo('/guid');
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
  const handleAssetLibraryClick = () => navTo(CREATIVE_STUDIO_ASSETS_PATH);
  const handleNomiClick = () => navTo('/nomi');
  const handleCreativeStudioClick = () => navTo(CREATIVE_STUDIO_PROJECTS_PATH);
  const handleMiniAppsClick = () => navTo('/mini-apps');
  const handleCustomerServiceClick = () => navTo('/customer-service');
  const handlePresetClick = () => navTo('/presets');
  const handleSkillsClick = () => navTo('/skills');
  const handleMcpClick = () => navTo('/mcp');
  const handleOpenCapabilitiesClick = () => navTo('/open-capabilities');
  const handleModelHubClick = () => navTo('/models');
  const handleCreativeStudioNavigation = useCallback(
    async (target: string, replace = false) => {
      if (!(await requestCreativeStudioBeforeLeave())) return;
      navTo(target, replace);
    },
    [navTo]
  );
  const handleReturnToWorkbench = useCallback(() => {
    void handleCreativeStudioNavigation(WORKBENCH_HOME_PATH, true);
  }, [handleCreativeStudioNavigation]);

  const handleSettingsClick = () => {
    cleanupSiderTooltips();
    blurActiveElement();
    if (isSettings) {
      const target = lastNonSettingsPathRef.current || '/guid';
      Promise.resolve(navigate(target)).catch((error) => {
        console.error('Navigation failed:', error);
      });
    } else {
      Promise.resolve(navigate('/settings/system')).catch((error) => {
        console.error('Navigation failed:', error);
      });
    }
    if (onSessionClick) {
      onSessionClick();
    }
  };

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

  const tooltipEnabled = collapsed && !isMobile;
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
        ) : isCreativeStudio ? (
          <Suspense fallback={<div className='size-full' />}>
            <CreativeStudioSider
              collapsed={collapsed}
              tooltipEnabled={tooltipEnabled}
              onNavigate={(target) => void handleCreativeStudioNavigation(target)}
            />
          </Suspense>
        ) : (
          <div className='size-full flex flex-col gap-1px'>
            {/* 常用 — high-frequency primary destinations */}
            <SiderSectionHeader label={t('common.siderSection.common')} collapsed={collapsed} />
            {/* Conversations — opens the session secondary sidebar (ContentSider) */}
            <SiderConversationEntry
              isMobile={isMobile}
              isActive={isSessionRoute}
              collapsed={collapsed}
              siderTooltipProps={siderTooltipProps}
              onClick={handleConversationClick}
            />
            {/* Work partner (桌面伙伴) */}
            <SiderNomiEntry
              isMobile={isMobile}
              isActive={pathname.startsWith('/nomi')}
              collapsed={collapsed}
              siderTooltipProps={siderTooltipProps}
              onClick={handleNomiClick}
            />
            {/* Creative Studio (创意工坊) — swaps this rail to product navigation. */}
            <SiderCreativeStudioEntry
              isMobile={isMobile}
              isActive={isCreativeStudio}
              collapsed={collapsed}
              siderTooltipProps={siderTooltipProps}
              onClick={handleCreativeStudioClick}
            />
            {/* 小程序 (Mini-apps) — solidified single-file web tools, opened instantly */}
            <SiderMiniAppsEntry
              isMobile={isMobile}
              isActive={pathname.startsWith('/mini-apps')}
              collapsed={collapsed}
              siderTooltipProps={siderTooltipProps}
              onClick={handleMiniAppsClick}
            />
            {/* 数据空间 — data & storage (文件管理 reserved for later) */}
            <SiderSectionHeader label={t('common.siderSection.data')} collapsed={collapsed} />
            {/* Knowledge base */}
            <SiderKnowledgeEntry
              isMobile={isMobile}
              isActive={pathname.startsWith('/knowledge')}
              collapsed={collapsed}
              siderTooltipProps={siderTooltipProps}
              onClick={handleKnowledgeClick}
            />
            {/* Asset library — enter the canonical Creative Studio surface. */}
            <SiderAssetLibraryEntry
              isMobile={isMobile}
              isActive={pathname === CREATIVE_STUDIO_ASSETS_PATH}
              collapsed={collapsed}
              siderTooltipProps={siderTooltipProps}
              onClick={handleAssetLibraryClick}
            />
            {/* 自动化 — automation platforms */}
            <SiderSectionHeader label={t('common.siderSection.automation')} collapsed={collapsed} />
            {/* Scheduled tasks */}
            <SiderScheduledEntry
              isMobile={isMobile}
              isActive={pathname === '/scheduled'}
              collapsed={collapsed}
              siderTooltipProps={siderTooltipProps}
              onClick={handleScheduledClick}
            />
            {/* Requirements platform */}
            <SiderRequirementsEntry
              isMobile={isMobile}
              isActive={pathname.startsWith('/requirements')}
              collapsed={collapsed}
              siderTooltipProps={siderTooltipProps}
              onClick={handleRequirementsClick}
            />
            {/* 增强工具 — extension capabilities */}
            <SiderSectionHeader label={t('common.siderSection.tools')} collapsed={collapsed} />
            {/* Presets and skills are separate concepts and destinations. */}
            <SiderPresetEntry
              isMobile={isMobile}
              isActive={pathname.startsWith('/presets')}
              collapsed={collapsed}
              siderTooltipProps={siderTooltipProps}
              onClick={handlePresetClick}
            />
            <SiderSkillsEntry
              isMobile={isMobile}
              isActive={pathname.startsWith('/skills')}
              collapsed={collapsed}
              siderTooltipProps={siderTooltipProps}
              onClick={handleSkillsClick}
            />
            {/* MCP — MCP tool server configuration */}
            <SiderMcpEntry
              isMobile={isMobile}
              isActive={pathname.startsWith('/mcp')}
              collapsed={collapsed}
              siderTooltipProps={siderTooltipProps}
              onClick={handleMcpClick}
            />
            {/* 服务 — public-facing services (客服), a domain fully separate
                from the desktop-companion group above. */}
            <SiderSectionHeader label={t('common.siderSection.services')} collapsed={collapsed} />
            <SiderCustomerServiceEntry
              isMobile={isMobile}
              isActive={pathname.startsWith('/customer-service')}
              collapsed={collapsed}
              siderTooltipProps={siderTooltipProps}
              onClick={handleCustomerServiceClick}
            />
          </div>
        )}
      </div>
      {/* Bottom pinned group — Creative Studio keeps its workbench exit here; other routes keep Settings. */}
      <div className='shrink-0 mt-auto pt-5px flex flex-col gap-1px border-t border-solid border-[var(--color-border-2)] border-l-0 border-r-0 border-b-0'>
        {isCreativeStudio ? (
          <SiderFooter
            isMobile={isMobile}
            isSettings={false}
            collapsed={collapsed}
            siderTooltipProps={siderTooltipProps}
            onSettingsClick={handleReturnToWorkbench}
            backLabel={t('creativeStudio.focus.backToWorkbench')}
          />
        ) : (
          <>
            {/* 设置 — section label; the enclosing border-t already separates this region when collapsed */}
            <SiderSectionHeader label={t('common.siderSection.settings')} collapsed={collapsed} collapsedRule={false} />
            {/* Unified Browser management — keep the entry reachable when Browser Use is
                disabled so the user can open Settings and turn it back on. */}
            {(isDesktopShell() || browserOverview?.supported !== false) && (
              <SiderBrowserEntry
                isMobile={isMobile}
                isActive={pathname === '/browser'}
                collapsed={collapsed}
                runningCount={browserOverview?.running_lanes ?? 0}
                queuedCount={browserOverview?.queued_lanes ?? 0}
                siderTooltipProps={siderTooltipProps}
                onClick={handleBrowserClick}
              />
            )}
            <SiderModelHubEntry
              isMobile={isMobile}
              isActive={pathname.startsWith('/models')}
              collapsed={collapsed}
              siderTooltipProps={siderTooltipProps}
              onClick={handleModelHubClick}
            />
            <SiderOpenCapabilitiesEntry
              isMobile={isMobile}
              isActive={pathname.startsWith('/open-capabilities')}
              collapsed={collapsed}
              siderTooltipProps={siderTooltipProps}
              onClick={handleOpenCapabilitiesClick}
            />
            <SiderFooter
              isMobile={isMobile}
              isSettings={isSettings}
              collapsed={collapsed}
              siderTooltipProps={siderTooltipProps}
              onSettingsClick={handleSettingsClick}
              showLogout={showLogout}
              onLogoutClick={handleLogout}
            />
          </>
        )}
      </div>
    </div>
  );
};

export default Sider;
