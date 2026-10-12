import React from 'react';
import { useTranslation } from 'react-i18next';
import { Application } from '@icon-park/react';
import SiderNavItem, { type SiderNavItemProps } from './SiderNavItem';

type SiderAppEntryProps = Pick<SiderNavItemProps, 'isActive' | 'collapsed' | 'siderTooltipProps' | 'onClick'>;

/** Hosted application entry in the shared desktop navigation rail. */
const SiderAppEntry: React.FC<SiderAppEntryProps> = ({ collapsed = false, ...props }) => {
  const { t } = useTranslation();
  return <SiderNavItem {...props} collapsed={collapsed} label={t('appHosting.nav')}
    icon={<Application theme='outline' size={collapsed ? 20 : 16} fill='currentColor' />} />;
};

export default SiderAppEntry;
