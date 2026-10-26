import { useTranslation } from 'react-i18next';
import { Briefcase } from '@icon-park/react';
import SiderNavItem, { type SiderNavItemProps } from './SiderNavItem';

type Props = Pick<SiderNavItemProps, 'isActive' | 'collapsed' | 'siderTooltipProps' | 'onClick'>;

/** Cloud-office work has a separate owner from local Office file previews. */
export default function SiderOfficeEntry({ collapsed = false, ...props }: Props) {
  const { t } = useTranslation();
  return <SiderNavItem {...props} collapsed={collapsed} label={t('office.title')}
    icon={<Briefcase theme='outline' size={collapsed ? 20 : 16} fill='currentColor' />} />;
}
