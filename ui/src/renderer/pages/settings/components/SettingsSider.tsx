import FlexFullContainer from '@/renderer/components/layout/FlexFullContainer';
import { resolveExtensionAssetUrl } from '@/renderer/utils/platform';
import { type IExtensionSettingsTab } from '@/common/adapter/ipcBridge';
import { useExtI18n } from '@/renderer/hooks/system/useExtI18n';
import { useExtensionSettingsTabs } from '@/renderer/hooks/system/useExtensionSettingsTabs';
import {
  Computer,
  Cpu,
  History,
  Info,
  Puzzle,
  Server,
  System,
} from '@icon-park/react';
import classNames from 'classnames';
import React, { useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { useLocation, useNavigate } from 'react-router-dom';
import { Tooltip } from '@arco-design/web-react';
import { getSiderTooltipProps } from '@/renderer/utils/ui/siderTooltip';
import { buildSettingsNavItems } from './settingsNavigation';

/** Builtin settings tab IDs in display order (must match router paths). */
export const BUILTIN_TAB_IDS = [
  'system',
  'execution-engines',
  'ssh-hosts',
  'computer-use',
  'computer-history',
  'about',
] as const;

/**
 * Group headers displayed above specific builtin tabs.
 * The header is rendered once, immediately before the first item whose id matches.
 * Extension tabs anchored between these builtins inherit the enclosing group visually.
 */
const GROUP_HEADER_BEFORE: Record<string, string> = {
  system: 'settings.groupApp',
  about: 'settings.groupAbout',
};

type SiderItem = {
  id: string;
  label: string;
  icon: React.ReactElement;
  isImageIcon?: boolean;
  /** Route path segment — for builtins: `/settings/{path}`, for extensions: `/settings/ext/{id}` */
  path: string;
};

const SettingsSider: React.FC<{ collapsed?: boolean; tooltipEnabled?: boolean }> = ({
  collapsed = false,
  tooltipEnabled = false,
}) => {
  const navigate = useNavigate();
  const { t } = useTranslation();
  const { pathname } = useLocation();

  const extensionTabs = useExtensionSettingsTabs();
  const { resolveExtTabName } = useExtI18n();

  const { menus, groupHeaderAt } = useMemo(() => {
    // Build builtin items
    const builtinMap: Record<string, SiderItem> = {
      'execution-engines': {
        id: 'execution-engines',
        label: t('settings.executionEngineHub.railTitle'),
        icon: <Cpu />,
        path: 'execution-engines',
      },
      'ssh-hosts': {
        id: 'ssh-hosts',
        label: t('ssh.title'),
        icon: <Server />,
        path: 'ssh-hosts',
      },
      system: { id: 'system', label: t('settings.system'), icon: <System />, path: 'system' },
      'computer-use': {
        id: 'computer-use',
        label: t('settings.computerUseNav'),
        icon: <Computer />,
        path: 'computer-use',
      },
      'computer-history': {
        id: 'computer-history',
        label: t('computerHistory.navTitle'),
        icon: <History />,
        path: 'computer-history',
      },
      about: { id: 'about', label: t('settings.about'), icon: <Info />, path: 'about' },
    };

    // Start with ordered builtin IDs
    const builtins: SiderItem[] = BUILTIN_TAB_IDS.map((id) => builtinMap[id]);

    // Helper to create SiderItem from extension tab
    const toSiderItem = (tab: IExtensionSettingsTab): SiderItem => {
      const resolvedIcon = resolveExtensionAssetUrl(tab.icon) || tab.icon;
      return {
        id: tab.id,
        label: resolveExtTabName(tab),
        icon: resolvedIcon ? <img src={resolvedIcon} alt='' className='w-full h-full object-contain' /> : <Puzzle />,
        isImageIcon: Boolean(resolvedIcon),
        path: `ext/${tab.id}`,
      };
    };

    const { items: result, beforeCounts } = buildSettingsNavItems(builtins, extensionTabs, toSiderItem);

    // Compute group header render positions.
    //
    // A header must appear before the first *visible* item of its group, which may
    // be an extension tab anchored with placement='before' to the group's first
    // builtin — not the builtin itself. Otherwise such an extension would render
    // above the header and visually belong to the previous group.
    const headerAt = new Map<number, string>();
    for (const [builtinId, headerKey] of Object.entries(GROUP_HEADER_BEFORE)) {
      const builtinIdx = result.findIndex((item) => item.id === builtinId);
      if (builtinIdx < 0) continue;
      const beforeCount = beforeCounts.get(builtinId) ?? 0;
      headerAt.set(builtinIdx - beforeCount, headerKey);
    }

    return { menus: result, groupHeaderAt: headerAt };
  }, [t, extensionTabs, resolveExtTabName]);

  const siderTooltipProps = getSiderTooltipProps(tooltipEnabled);
  return (
    <div
      className={classNames('h-full settings-sider flex flex-col gap-2px overflow-y-auto overflow-x-hidden', {
        'settings-sider--collapsed': collapsed,
      })}
    >
      {menus.map((item, index) => {
        const isSelected = pathname.includes(item.path);
        const groupHeaderKey = groupHeaderAt.get(index);
        const groupHeader =
          groupHeaderKey && !collapsed ? (
            <div className='settings-sider__group-header px-12px mt-8px h-28px flex items-center text-14px font-[500] text-t-tertiary select-none'>
              {t(groupHeaderKey)}
            </div>
          ) : null;
        return (
          <React.Fragment key={item.id}>
            {groupHeader}
            <Tooltip {...siderTooltipProps} content={item.label} position='right'>
              <div
                data-settings-id={item.id}
                data-settings-path={item.path}
                className={classNames(
                  'settings-sider__item h-34px rd-8px flex items-center gap-8px group cursor-pointer relative overflow-hidden shrink-0 conversation-item [&.conversation-item+&.conversation-item]:mt-2px transition-colors',
                  collapsed ? 'w-full justify-center px-0' : 'justify-start px-10px',
                  {
                    'hover:bg-fill-2': !isSelected,
                    '!bg-primary-1 !text-primary-6': isSelected,
                  }
                )}
                onClick={() => {
                  Promise.resolve(navigate(`/settings/${item.path}`, { replace: true })).catch((error) => {
                    console.error('Navigation failed:', error);
                  });
                }}
              >
                {/* Leading icon — 22px slot to align with main sider rows */}
                <span className='size-22px flex items-center justify-center shrink-0 line-height-0'>
                  {item.isImageIcon ? (
                    <span className='w-16px h-16px flex items-center justify-center'>{item.icon}</span>
                  ) : (
                    React.cloneElement(
                      item.icon as React.ReactElement<{
                        theme?: string;
                        size?: string | number;
                        className?: string;
                        strokeWidth?: number;
                      }>,
                      {
                        theme: 'outline',
                        size: '16',
                        strokeWidth: 3,
                        className: isSelected ? 'block leading-none text-primary-6' : 'block leading-none text-t-secondary',
                      }
                    )
                  )}
                </span>
                <FlexFullContainer className='h-24px collapsed-hidden'>
                  <div className={classNames('settings-sider__item-label text-nowrap overflow-hidden inline-block w-full text-14px font-[500] lh-24px whitespace-nowrap', isSelected ? 'text-primary-6' : 'text-t-primary')}>
                    {item.label}
                  </div>
                </FlexFullContainer>
              </div>
            </Tooltip>
          </React.Fragment>
        );
      })}
    </div>
  );
};

export default SettingsSider;
