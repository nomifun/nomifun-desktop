import classNames from 'classnames';
import React from 'react';
import { useLayoutContext } from '@/renderer/hooks/context/LayoutContext';
import { resolveExtensionAssetUrl } from '@/renderer/utils/platform';
import { type IExtensionSettingsTab } from '@/common/adapter/ipcBridge';
import { useExtensionSettingsTabs } from '@/renderer/hooks/system/useExtensionSettingsTabs';
import { Computer, Cpu, History, Info, Puzzle, Server, System } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import { useLocation, useNavigate } from 'react-router-dom';
import { useExtI18n } from '@/renderer/hooks/system/useExtI18n';
import { BUILTIN_TAB_IDS } from './SettingsSider';
import { buildSettingsNavItems } from './settingsNavigation';
import './settings.css';

interface SettingsPageWrapperProps {
  children: React.ReactNode;
  className?: string;
  contentClassName?: string;
}

type NavItem = { label: string; icon: React.ReactElement; path: string; id: string };

type TranslateFn = (key: string, options?: { defaultValue?: string }) => string;

export function getBuiltinSettingsNavItems(t: TranslateFn): NavItem[] {
  const builtinMap: Record<string, NavItem> = {
    'execution-engines': {
      id: 'execution-engines',
      label: t('settings.executionEngineHub.railTitle'),
      icon: <Cpu theme='outline' size='16' />,
      path: 'execution-engines',
    },
    'ssh-hosts': {
      id: 'ssh-hosts',
      label: t('ssh.title'),
      icon: <Server theme='outline' size='16' />,
      path: 'ssh-hosts',
    },
    system: { id: 'system', label: t('settings.system'), icon: <System theme='outline' size='16' />, path: 'system' },
    'computer-use': {
      id: 'computer-use',
      label: t('settings.computerUseNav'),
      icon: <Computer theme='outline' size='16' />,
      path: 'computer-use',
    },
    'computer-history': {
      id: 'computer-history',
      label: t('computerHistory.navTitle'),
      icon: <History theme='outline' size='16' />,
      path: 'computer-history',
    },
    about: { id: 'about', label: t('settings.about'), icon: <Info theme='outline' size='16' />, path: 'about' },
  };

  return BUILTIN_TAB_IDS.map((id) => builtinMap[id]);
}

const SettingsPageWrapper: React.FC<SettingsPageWrapperProps> = ({ children, className, contentClassName }) => {
  const layout = useLayoutContext();
  const isMobile = layout?.isMobile ?? false;
  const navigate = useNavigate();
  const { pathname } = useLocation();
  const { t } = useTranslation();

  const extensionTabs = useExtensionSettingsTabs();

  const { resolveExtTabName } = useExtI18n();

  const menuItems = React.useMemo(() => {
    const builtins = getBuiltinSettingsNavItems(t);

    const toNavItem = (tab: IExtensionSettingsTab): NavItem => {
      const resolvedIcon = resolveExtensionAssetUrl(tab.icon) || tab.icon;
      return {
        id: tab.id,
        label: resolveExtTabName(tab),
        icon: resolvedIcon ? (
          <img src={resolvedIcon} alt='' className='w-16px h-16px object-contain' />
        ) : (
          <Puzzle theme='outline' size='16' />
        ),
        path: `ext/${tab.id}`,
      };
    };

    // Insert extension tabs at their anchor, or (unanchored) at the end of the
    // "Application" group — before "about" — to keep them inside that group.
    return buildSettingsNavItems(builtins, extensionTabs, toNavItem).items;
  }, [t, extensionTabs, resolveExtTabName]);

  const containerClass = classNames(
    'settings-page-wrapper w-full min-h-full box-border overflow-y-auto',
    isMobile ? 'px-16px py-14px' : 'px-12px md:px-40px py-32px',
    className
  );

  const contentClass = classNames('settings-page-content mx-auto w-full md:max-w-1024px', contentClassName);

  return (
    <div className={containerClass}>
      {isMobile && (
        <div className='settings-mobile-top-nav'>
          {menuItems.map((item) => {
            const active = pathname.includes(`/settings/${item.path}`);
            return (
              <button
                key={item.path}
                type='button'
                className={classNames('settings-mobile-top-nav__item', {
                  'settings-mobile-top-nav__item--active': active,
                })}
                onClick={() => {
                  void navigate(`/settings/${item.path}`, { replace: true });
                }}
              >
                <span className='settings-mobile-top-nav__icon'>{item.icon}</span>
                <span className='settings-mobile-top-nav__label'>{item.label}</span>
              </button>
            );
          })}
        </div>
      )}
      <div className={contentClass}>{children}</div>
    </div>
  );
};

export default SettingsPageWrapper;
