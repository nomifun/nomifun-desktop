import React, { useEffect, useMemo, useState } from 'react';
import classNames from 'classnames';
import { ArrowLeft, ArrowRight, ExpandLeft, ExpandRight, Plus, Terminal } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import { useLocation, useNavigate } from 'react-router-dom';

import { ipcBridge } from '@/common';
import InstantHoverTooltip from '@renderer/components/base/InstantHoverTooltip';
import TitlebarLanguageMenu from './TitlebarLanguageMenu';
import WindowControls from '../WindowControls';
import {
  SESSION_SIDER_STATE_EVENT,
  dispatchSessionSiderToggleEvent,
} from '@renderer/utils/workspace/sessionSiderEvents';
import type { SessionSiderStateDetail } from '@renderer/utils/workspace/sessionSiderEvents';
import { AGENT_SIDER_STATE_EVENT, dispatchAgentSiderToggleEvent } from '@renderer/utils/workspace/agentSiderEvents';
import { useLayoutContext } from '@/renderer/hooks/context/LayoutContext';
import { useNavigationHistory } from '@/renderer/hooks/context/NavigationHistoryContext';
import { isDesktopShell, isMacOS } from '@/renderer/utils/platform';
import { requestCreativeStudioBeforeLeave } from '@renderer/pages/creativeStudio/app/beforeLeave';
import './titlebar.css';

type TitlebarIconButtonOptions = {
  tooltip: string;
  className: string;
  children: React.ReactNode;
  disabled?: boolean;
  onClick?: () => void;
};

// Claude-desktop-style sidebar toggle icon: a rounded rectangle with a vertical divider
// near the left edge, indicating a collapsible side panel. Rendered as inline SVG since
// @icon-park doesn't ship this exact shape.
//
// Uses a 48-unit viewBox to match @icon-park's stroke scale, so passing the same
// `strokeWidth` value here and to @icon-park icons produces visually identical lines.
//
// The rect spans y=10..38 (height 28), slightly taller than @icon-park's
// ArrowLeft/ArrowRight (which span y=12..36) so the sidebar icon reads a
// touch larger. The rect remains centered at y=24, matching the arrows'
// centerline so all three icons stay on the same visual baseline.
const SidebarIcon: React.FC<{ size?: number; strokeWidth?: number }> = ({ size = 18, strokeWidth = 4 }) => (
  <svg
    width={size}
    height={size}
    viewBox='0 0 48 48'
    fill='none'
    stroke='currentColor'
    strokeWidth={strokeWidth}
    strokeLinecap='round'
    strokeLinejoin='round'
    aria-hidden='true'
    focusable='false'
  >
    <rect x='6' y='10' width='36' height='28' rx='5' />
    <line x1='18' y1='10' x2='18' y2='38' />
  </svg>
);

const Titlebar: React.FC = () => {
  const { t } = useTranslation();
  const appTitle = useMemo(() => 'NomiFun', []);
  const [sessionSiderCollapsed, setSessionSiderCollapsed] = useState(false);
  const [agentSiderCollapsed, setAgentSiderCollapsed] = useState(false);
  useEffect(() => {
    const handler = (event: Event) => setAgentSiderCollapsed((event as CustomEvent<{ collapsed: boolean }>).detail.collapsed);
    window.addEventListener(AGENT_SIDER_STATE_EVENT, handler);
    return () => window.removeEventListener(AGENT_SIDER_STATE_EVENT, handler);
  }, []);
  const layout = useLayoutContext();
  const navigationHistory = useNavigationHistory();
  const location = useLocation();
  const navigate = useNavigate();
  // 同步会话二级侧栏折叠状态，使标题栏开关图标保持一致
  // Sync session secondary-sidebar collapsed state for the titlebar toggle icon
  useEffect(() => {
    if (typeof window === 'undefined') {
      return undefined;
    }
    const handler = (event: Event) => {
      const customEvent = event as CustomEvent<SessionSiderStateDetail>;
      if (typeof customEvent.detail?.collapsed === 'boolean') {
        setSessionSiderCollapsed(customEvent.detail.collapsed);
      }
    };
    window.addEventListener(SESSION_SIDER_STATE_EVENT, handler as EventListener);
    return () => {
      window.removeEventListener(SESSION_SIDER_STATE_EVENT, handler as EventListener);
    };
  }, []);

  const isDesktopRuntime = isDesktopShell();
  const isMacRuntime = isDesktopRuntime && isMacOS();
  // Windows/Linux 显示自定义窗口按钮。
  const showWindowControls = isDesktopRuntime && !isMacRuntime;
  const iconSize = 18;
  const desktopIconStroke = 2.5;
  // 统一在标题栏左侧展示主侧栏开关 / Always expose sidebar toggle on titlebar left side
  const showSiderToggle = Boolean(layout?.setSiderCollapsed);
  const siderTooltip = layout?.siderCollapsed
    ? t('common.expandMore', { defaultValue: 'Expand sidebar' })
    : t('common.collapse', { defaultValue: 'Collapse sidebar' });
  const showHistoryNav = Boolean(navigationHistory);
  const historyBackTooltip = t('common.historyBack', { defaultValue: 'Back' });
  const historyForwardTooltip = t('common.forward', { defaultValue: 'Forward' });
  // The session secondary-sidebar toggle is shown on session routes only.
  const isSessionRoute =
    location.pathname === '/guid' ||
    location.pathname.startsWith('/conversation/') ||
    location.pathname === '/terminal-new' ||
    location.pathname.startsWith('/terminal/');
  const sessionToggleTooltip = sessionSiderCollapsed
    ? t('sessionList.expandList', { defaultValue: 'Show conversations' })
    : t('sessionList.collapseList', { defaultValue: 'Hide conversations' });
  const isAgentRoute = location.pathname === '/agent';
  const contentSiderCollapsed = isAgentRoute ? agentSiderCollapsed : sessionSiderCollapsed;
  const contentSiderTooltip = isAgentRoute
    ? t(agentSiderCollapsed ? 'agentSettings.workbench.showList' : 'agentSettings.workbench.hideList')
    : sessionToggleTooltip;

  const handleSiderToggle = () => {
    if (!showSiderToggle || !layout?.setSiderCollapsed) return;
    layout.setSiderCollapsed(!layout.siderCollapsed);
  };

  const navigateAfterCreativeStudioFlush = (action: () => void) => {
    void (async () => {
      if (!(await requestCreativeStudioBeforeLeave())) return;
      action();
    })();
  };

  // Windows/Linux: double-clicking the titlebar drag region toggles maximize,
  // matching native window behavior. Tauri's `data-tauri-drag-region` does NOT
  // implement this itself; we wire it on the frontend. Skipped on macOS (the OS
  // handles double-click on the native traffic-light chrome) and in the WebUI
  // browser (no window controls — `isDesktopRuntime` gates it). Only fires when
  // the double-click lands on the drag region itself, not on a `no-drag` button
  // (those carry `data-tauri-drag-region` absence + their own handlers).
  const handleTitlebarDoubleClick = (event: React.MouseEvent<HTMLDivElement>) => {
    if (!isDesktopRuntime || isMacRuntime) return;
    const target = event.target as HTMLElement | null;
    if (!target || !target.hasAttribute('data-tauri-drag-region')) return;
    void ipcBridge.windowControls.toggleMaximize.invoke();
  };

  const menuStyle: React.CSSProperties = useMemo(() => {
    if (!isMacRuntime || !showSiderToggle) return {};
    // macOS: sit the menu buttons right next to the traffic lights (which occupy ~70px).
    return {
      marginLeft: '76px',
    };
  }, [isMacRuntime, showSiderToggle]);

  const renderIconButton = ({ tooltip, className, children, disabled, onClick }: TitlebarIconButtonOptions) => (
    <InstantHoverTooltip content={tooltip} position='bottom'>
      <button type='button' className={className} onClick={onClick} disabled={disabled} aria-label={tooltip}>
        {children}
      </button>
    </InstantHoverTooltip>
  );

  return (
    <div
      data-tauri-drag-region
      onDoubleClick={handleTitlebarDoubleClick}
      // 标题栏底部分隔线：原来只有 border-b（宽度）+ 边框色，没有 border-style，
      // 而本仓库没有全局 border reset，所以这条线从来没画出来过。
      // The titlebar's bottom rule never painted: width + colour but no border-style.
      className={classNames(
        'flex items-center gap-8px app-titlebar bg-2 border-b border-b-solid border-[var(--border-base)]',
        {
          'app-titlebar--desktop': isDesktopRuntime,
          'app-titlebar--mac': isMacRuntime,
        },
      )}
    >
      <div className='app-titlebar__menu' style={menuStyle}>
        {showSiderToggle && (
          renderIconButton({
            tooltip: siderTooltip,
            className: 'app-titlebar__button',
            onClick: handleSiderToggle,
            children: <SidebarIcon size={iconSize} strokeWidth={desktopIconStroke} />,
          })
        )}
        {showHistoryNav && (
          <>
            {renderIconButton({
              tooltip: historyBackTooltip,
              className: 'app-titlebar__button app-titlebar__button--nav',
              onClick: () => navigateAfterCreativeStudioFlush(() => navigationHistory?.back()),
              disabled: !navigationHistory?.canBack,
              children: <ArrowLeft theme='outline' size={iconSize} fill='currentColor' strokeWidth={desktopIconStroke} />,
            })}
            {renderIconButton({
              tooltip: historyForwardTooltip,
              className: 'app-titlebar__button app-titlebar__button--nav',
              onClick: () => navigateAfterCreativeStudioFlush(() => navigationHistory?.forward()),
              disabled: !navigationHistory?.canForward,
              children: <ArrowRight theme='outline' size={iconSize} fill='currentColor' strokeWidth={desktopIconStroke} />,
            })}
          </>
        )}
        {renderIconButton({
          tooltip: t('terminal.newConversation'),
          className: 'app-titlebar__button app-titlebar__button--nav',
          onClick: () =>
            navigateAfterCreativeStudioFlush(() => navigate('/guid', { state: { resetAgentSelection: true } })),
          children: <Plus theme='outline' size={iconSize} fill='currentColor' strokeWidth={desktopIconStroke} />,
        })}
        {renderIconButton({
          tooltip: t('terminal.newTerminal'),
          className: 'app-titlebar__button app-titlebar__button--nav',
          onClick: () => navigateAfterCreativeStudioFlush(() => navigate('/terminal-new')),
          children: <Terminal theme='outline' size={iconSize} fill='currentColor' strokeWidth={desktopIconStroke} />,
        })}
        {(isSessionRoute || isAgentRoute) && (
          renderIconButton({
            tooltip: contentSiderTooltip,
            className: 'app-titlebar__button app-titlebar__button--nav',
            onClick: () => isAgentRoute ? dispatchAgentSiderToggleEvent() : dispatchSessionSiderToggleEvent(),
            children: contentSiderCollapsed ? (
              <ExpandRight theme='outline' size={iconSize} fill='currentColor' strokeWidth={desktopIconStroke} />
            ) : (
              <ExpandLeft theme='outline' size={iconSize} fill='currentColor' strokeWidth={desktopIconStroke} />
            ),
          })
        )}
        <div className='app-titlebar__language-control'>
          <TitlebarLanguageMenu strokeWidth={desktopIconStroke} />
        </div>
      </div>
      <div
        className={classNames('app-titlebar__brand', {
          'app-titlebar__brand--centered': !location.pathname.match(/^\/conversation\//),
        })}
        aria-label={appTitle}
        title={appTitle}
      />
      <div className='app-titlebar__toolbar'>
        {showWindowControls && <WindowControls />}
      </div>
    </div>
  );
};

export default Titlebar;
