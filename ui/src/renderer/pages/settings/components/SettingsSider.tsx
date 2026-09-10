import FlexFullContainer from '@/renderer/components/layout/FlexFullContainer';
import {
  Computer,
  Cpu,
  Info,
  Server,
  System,
} from '@icon-park/react';
import classNames from 'classnames';
import React, { useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { useLocation, useNavigate } from 'react-router-dom';
import { Tooltip } from '@arco-design/web-react';
import { getSiderTooltipProps } from '@/renderer/utils/ui/siderTooltip';

/** Builtin settings tab IDs in display order (must match router paths). */
export const BUILTIN_TAB_IDS = [
  'system',
  'execution-engines',
  'ssh-hosts',
  'computer-use',
  'about',
] as const;

/**
 * Group headers displayed above specific built-in tabs.
 * Each header is rendered immediately before its matching item.
 */
const GROUP_HEADER_BEFORE: Record<string, string> = {
  system: 'settings.groupApp',
  about: 'settings.groupAbout',
};

type SiderItem = {
  id: string;
  label: string;
  icon: React.ReactElement;
  /** Route path segment for the built-in settings page. */
  path: string;
};

const SettingsSider: React.FC<{ collapsed?: boolean; tooltipEnabled?: boolean }> = ({
  collapsed = false,
  tooltipEnabled = false,
}) => {
  const navigate = useNavigate();
  const { t } = useTranslation();
  const { pathname } = useLocation();

  const { menus, groupHeaderAt } = useMemo(() => {
    const builtinMap: Record<string, SiderItem> = {
      'execution-engines': {
        id: 'execution-engines',
        label: t('settings.runtimeManager.railTitle'),
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
      about: { id: 'about', label: t('settings.about'), icon: <Info />, path: 'about' },
    };

    const builtins: SiderItem[] = BUILTIN_TAB_IDS.map((id) => builtinMap[id]);
    const headerAt = new Map<number, string>();
    for (const [builtinId, headerKey] of Object.entries(GROUP_HEADER_BEFORE)) {
      const builtinIdx = builtins.findIndex((item) => item.id === builtinId);
      if (builtinIdx >= 0) headerAt.set(builtinIdx, headerKey);
    }

    return { menus: builtins, groupHeaderAt: headerAt };
  }, [t]);

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
                  {React.cloneElement(
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
