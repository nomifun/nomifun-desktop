import { Cpu, Info, Server, Shield, System } from '@icon-park/react';
import React from 'react';
import { useTranslation } from 'react-i18next';
import { getSiderTooltipProps } from '@/renderer/utils/ui/siderTooltip';
import SiderNavItem from '@/renderer/components/layout/Sider/SiderNav/SiderNavItem';
import SiderSectionHeader from '@/renderer/components/layout/Sider/SiderNav/SiderSectionHeader';

export const BUILTIN_TAB_IDS = ['system', 'permissions', 'execution-engines', 'ssh-hosts', 'about'] as const;

/** Settings destinations use the same rows and group labels as the app rail. */
const SettingsSider: React.FC<{ collapsed?: boolean; tooltipEnabled?: boolean }> = ({ collapsed = false, tooltipEnabled = false }) => {
  const { t } = useTranslation();
  const items = [
    { id: 'system', label: t('settings.workspace.generalTitle'), icon: <System theme='outline' size={16} /> },
    { id: 'permissions', label: t('settings.capabilityPermissions.nav'), icon: <Shield theme='outline' size={16} /> },
    { id: 'execution-engines', label: t('settings.executionEngines.title'), icon: <Cpu theme='outline' size={16} /> },
    { id: 'ssh-hosts', label: t('ssh.title'), icon: <Server theme='outline' size={16} /> },
    { id: 'about', label: t('settings.about'), icon: <Info theme='outline' size={16} /> },
  ];
  return <nav aria-label={t('settings.title')} className='h-full flex flex-col gap-1px overflow-y-auto'>
    <SiderSectionHeader label={t('settings.title')} collapsed={collapsed} collapsedRule={false} />
    {items.map((item) => <React.Fragment key={item.id}>
      {item.id === 'execution-engines' && <SiderSectionHeader label={t('settings.groupRuntimes')} collapsed={collapsed} />}
      {item.id === 'about' && <SiderSectionHeader label={t('settings.groupAbout')} collapsed={collapsed} />}
      <SiderNavItem to={`/settings/${item.id}`} replace label={item.label} icon={item.icon} collapsed={collapsed}
        siderTooltipProps={getSiderTooltipProps(tooltipEnabled)} data-settings-id={item.id} data-settings-path={item.id} />
    </React.Fragment>)}
  </nav>;
};

export default SettingsSider;
