import { Cpu, DatabaseConfig, Info, Server, Shield, System } from '@icon-park/react';
import React from 'react';
import { useTranslation } from 'react-i18next';
import { useLocation, useNavigate } from 'react-router-dom';
import { getSiderTooltipProps } from '@/renderer/utils/ui/siderTooltip';
import SiderNavItem from '@/renderer/components/layout/Sider/SiderNav/SiderNavItem';
import SiderSectionHeader from '@/renderer/components/layout/Sider/SiderNav/SiderSectionHeader';
import { generalSettingsHref, resolveGeneralSettingsSection, type GeneralSettingsSection } from './generalSettingsNavigation';

export const BUILTIN_TAB_IDS = ['system', 'permissions', 'execution-engines', 'ssh-hosts', 'about'] as const;

/** Settings destinations use the same rows and group labels as the app rail. */
const SettingsSider: React.FC<{ collapsed?: boolean; tooltipEnabled?: boolean }> = ({ collapsed = false, tooltipEnabled = false }) => {
  const { t } = useTranslation();
  const location = useLocation();
  const navigate = useNavigate();
  const activeGeneralSection = location.pathname === '/settings/system'
    ? resolveGeneralSettingsSection(new URLSearchParams(location.search).get('section')) : undefined;
  const items: { id: string; label: string; icon: React.ReactNode; section?: GeneralSettingsSection }[] = [
    { id: 'preferences', section: 'preferences', label: t('settings.workspace.preferences'), icon: <System theme='outline' size={16} /> },
    { id: 'data', section: 'data', label: t('settings.workspace.data'), icon: <DatabaseConfig theme='outline' size={16} /> },
    { id: 'permissions', label: t('settings.capabilityPermissions.nav'), icon: <Shield theme='outline' size={16} /> },
    { id: 'execution-engines', label: t('settings.executionEngines.title'), icon: <Cpu theme='outline' size={16} /> },
    { id: 'ssh-hosts', label: t('ssh.title'), icon: <Server theme='outline' size={16} /> },
    { id: 'about', label: t('settings.about'), icon: <Info theme='outline' size={16} /> },
  ];
  return <nav aria-label={t('settings.title')} className='h-full flex flex-col gap-1px overflow-y-auto'>
    <SiderSectionHeader label={t('settings.title')} collapsed={collapsed} collapsedRule={false} />
    {items.map((item) => {
      const section = item.section;
      return <React.Fragment key={item.id}>
        {item.id === 'execution-engines' && <SiderSectionHeader label={t('settings.groupRuntimes')} collapsed={collapsed} />}
        {item.id === 'about' && <SiderSectionHeader label={t('settings.groupAbout')} collapsed={collapsed} />}
        <SiderNavItem to={section ? undefined : `/settings/${item.id}`} replace label={item.label} icon={item.icon} collapsed={collapsed}
          isActive={section !== undefined && section === activeGeneralSection}
          onClick={section ? () => navigate(generalSettingsHref(section, undefined, location.search), { replace: true }) : undefined}
          siderTooltipProps={getSiderTooltipProps(tooltipEnabled)} data-settings-id={item.id} data-settings-path={item.id} />
      </React.Fragment>;
    })}
  </nav>;
};

export default SettingsSider;
