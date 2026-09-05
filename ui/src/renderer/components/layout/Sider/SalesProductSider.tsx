import { Tooltip } from '@arco-design/web-react';
import {
  BuildingOne,
  ChartHistogram,
  CloseOne,
  DataScreen,
  Plan,
  Robot,
  SalesReport,
  SettingTwo,
  Shield,
  TableReport,
  Theme,
} from '@icon-park/react';
import classNames from 'classnames';
import React, { useCallback } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';
import { useAuth } from '@renderer/hooks/context/AuthContext';
import { useLayoutContext } from '@renderer/hooks/context/LayoutContext';
import { isDesktopShell } from '@renderer/utils/platform';
import { blurActiveElement } from '@renderer/utils/ui/focus';
import { cleanupSiderTooltips, getSiderTooltipProps } from '@renderer/utils/ui/siderTooltip';
import { useSalesOwnerAccess } from '@renderer/pages/sales/useSalesOwnerAccess';
import SiderThemeControl from './SiderThemeControl';

interface SalesProductSiderProps {
  onSessionClick?: () => void;
  collapsed?: boolean;
}

interface ProductIconProps {
  theme?: string;
  size?: string | number;
  strokeWidth?: number;
  'aria-hidden'?: boolean;
}

type ProductNavItem = {
  path: string;
  label: string;
  icon: React.ReactElement<ProductIconProps>;
  active(pathname: string): boolean;
};

const customerNavigation: ProductNavItem[] = [
  { path: '/sales', label: '工作台', icon: <DataScreen />, active: (path) => path === '/sales' },
  { path: '/sales/company', label: '我的公司', icon: <BuildingOne />, active: (path) => path === '/sales/company' },
  { path: '/sales/tasks', label: '联络任务', icon: <Plan />, active: (path) => path === '/sales/tasks' },
  { path: '/sales/companies', label: '目标公司', icon: <TableReport />, active: (path) => path === '/sales/companies' },
  { path: '/sales/approvals', label: '执行看板', icon: <ChartHistogram />, active: (path) => path === '/sales/approvals' },
  { path: '/sales/results', label: '结果记录', icon: <SalesReport />, active: (path) => path === '/sales/results' },
];

const accountNavigation: ProductNavItem = {
  path: '/sales/users',
  label: '账号与隔离',
  icon: <Shield />,
  active: (path) => path === '/sales/users',
};

const operatorNavigation: ProductNavItem[] = [
  {
    path: '/guid',
    label: 'Agent 会话',
    icon: <Robot />,
    active: (path) => path === '/guid' || path.startsWith('/conversation/'),
  },
  { path: '/models', label: '模型管理', icon: <DataScreen />, active: (path) => path.startsWith('/models') },
  { path: '/browser', label: '浏览器管理', icon: <TableReport />, active: (path) => path === '/browser' },
  { path: '/presets', label: 'Agent 预设', icon: <SettingTwo />, active: (path) => path.startsWith('/presets') },
  { path: '/skills', label: '销售技能', icon: <Theme />, active: (path) => path.startsWith('/skills') },
];

const SalesProductSider: React.FC<SalesProductSiderProps> = ({ onSessionClick, collapsed = false }) => {
  const { pathname } = useLocation();
  const navigate = useNavigate();
  const layout = useLayoutContext();
  const { logout, status } = useAuth();
  const { isInstanceOwner } = useSalesOwnerAccess();
  const isMobile = layout?.isMobile ?? false;
  const customerMode = pathname === '/sales' || pathname.startsWith('/sales/');
  const navigation = customerMode
    ? isInstanceOwner
      ? [...customerNavigation, accountNavigation]
      : customerNavigation
    : isInstanceOwner
      ? operatorNavigation
      : operatorNavigation.slice(0, 1);
  const tooltipProps = getSiderTooltipProps(collapsed && !isMobile);
  const showLogout = !isDesktopShell() && status === 'authenticated';

  const goTo = useCallback(
    (path: string) => {
      cleanupSiderTooltips();
      blurActiveElement();
      void navigate(path);
      onSessionClick?.();
    },
    [navigate, onSessionClick]
  );

  const handleLogout = useCallback(async () => {
    cleanupSiderTooltips();
    blurActiveElement();
    try {
      await logout();
      onSessionClick?.();
    } catch (error) {
      console.error('Logout failed:', error);
    }
  }, [logout, onSessionClick]);

  const footerButtonClass = classNames(
    'h-32px flex items-center rd-8px border-0 bg-transparent cursor-pointer transition-colors hover:bg-fill-2 text-t-primary',
    collapsed ? 'w-full justify-center px-0' : 'w-full justify-start gap-8px px-10px'
  );

  return (
    <nav className='size-full flex flex-col' aria-label={customerMode ? '销售工作台导航' : '运营管理导航'}>
      <div className='flex-1 min-h-0 overflow-y-auto overflow-x-hidden'>
        {!collapsed && (
          <div className='px-10px pt-4px pb-10px'>
            <div className='text-11px font-600 tracking-0.08em text-t-tertiary'>
              {customerMode ? '销售流程' : '运营管理'}
            </div>
          </div>
        )}
        <div className='flex flex-col gap-2px'>
          {navigation.map((item) => {
            const selected = item.active(pathname);
            return (
              <Tooltip key={item.path} {...tooltipProps} content={item.label} position='right'>
                <button
                  type='button'
                  className={classNames(
                    'h-34px rd-8px border-0 bg-transparent flex items-center gap-8px cursor-pointer transition-colors text-t-primary',
                    collapsed ? 'w-full justify-center px-0' : 'w-full justify-start px-10px',
                    selected ? '!bg-primary-1 !text-primary-6' : 'hover:bg-fill-2'
                  )}
                  aria-current={selected ? 'page' : undefined}
                  onClick={() => goTo(item.path)}
                >
                  <span className={classNames('size-22px flex items-center justify-center shrink-0', selected ? 'text-primary-6' : 'text-t-secondary')}>
                    {React.cloneElement(item.icon, {
                      theme: 'outline',
                      size: 16,
                      strokeWidth: 3,
                      'aria-hidden': true,
                    })}
                  </span>
                  <span className='collapsed-hidden text-14px font-500 leading-24px truncate'>{item.label}</span>
                </button>
              </Tooltip>
            );
          })}
        </div>
      </div>

      <div className='shrink-0 mt-auto pt-7px pb-5px border-t border-solid border-[var(--color-border-2)] border-l-0 border-r-0 border-b-0 flex flex-col gap-2px'>
        {customerMode && !collapsed && (
          <div className='mx-4px mb-5px px-8px py-7px rd-8px bg-fill-2 flex items-center gap-7px text-11px text-t-secondary'>
            <Shield size={14} className='shrink-0' aria-hidden='true' />
            <span>合格公司自动提交并留档</span>
          </div>
        )}
        {(!customerMode || isInstanceOwner) && (
          <Tooltip
            {...tooltipProps}
            content={customerMode ? '运营管理' : '返回销售工作台'}
            position='right'
          >
            <button type='button' className={footerButtonClass} onClick={() => goTo(customerMode ? '/admin' : '/sales')}>
              <span className='size-22px flex items-center justify-center shrink-0 text-t-secondary'>
                {customerMode ? <SettingTwo size={16} aria-hidden='true' /> : <SalesReport size={16} aria-hidden='true' />}
              </span>
              <span className='collapsed-hidden text-13px font-500 leading-24px truncate'>
                {customerMode ? '运营管理' : '返回销售工作台'}
              </span>
            </button>
          </Tooltip>
        )}
        <div className={classNames('flex gap-2px', collapsed ? 'flex-col' : 'items-center')}>
          <SiderThemeControl isMobile={isMobile} collapsed={collapsed} siderTooltipProps={tooltipProps} />
          {showLogout && (
            <Tooltip {...tooltipProps} content='退出登录' position='right'>
              <button type='button' className={footerButtonClass} onClick={() => void handleLogout()}>
                <span className='size-22px flex items-center justify-center shrink-0 text-t-secondary'>
                  <CloseOne size={16} aria-hidden='true' />
                </span>
                <span className='collapsed-hidden text-13px font-500 leading-24px truncate'>退出登录</span>
              </button>
            </Tooltip>
          )}
        </div>
      </div>
    </nav>
  );
};

export default SalesProductSider;
