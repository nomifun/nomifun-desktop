import { Tooltip } from '@arco-design/web-react';
import type { SiderTooltipProps } from '@renderer/utils/ui/siderTooltip';
import classNames from 'classnames';
import React from 'react';
import { NavLink } from 'react-router-dom';
import './SiderNav.css';

export interface SiderNavItemProps extends React.AriaAttributes {
  label: string;
  icon?: React.ReactNode;
  isActive?: boolean;
  collapsed?: boolean;
  siderTooltipProps?: SiderTooltipProps;
  className?: string;
  id?: string;
  role?: React.AriaRole;
  tabIndex?: number;
  onKeyDown?: React.KeyboardEventHandler<HTMLElement>;
  onClick?: () => void;
  to?: string;
  replace?: boolean;
}

/** One compact row for the app rail, settings routes and workspace tabs. */
export default function SiderNavItem({ label, icon, isActive = false, collapsed = false, siderTooltipProps,
  className, to, replace, onClick, role, ...attributes }: SiderNavItemProps) {
  const rowClass = (active: boolean) => classNames('sider-nav-item', className, {
    'sider-nav-item--active': active, 'sider-nav-item--collapsed': collapsed,
  });
  const content = <>
    {icon && <span className='sider-nav-item__icon' aria-hidden='true'>{icon}</span>}
    {!collapsed && <span className='sider-nav-item__label'>{label}</span>}
  </>;
  const row = to ? <NavLink {...attributes} to={to} replace={replace} onClick={onClick} role={role}
    aria-label={label} className={({ isActive: routeActive }) => rowClass(routeActive)}>{content}</NavLink>
    : <button {...attributes} type='button' role={role} aria-label={label}
      aria-current={role === 'tab' ? undefined : isActive ? 'page' : undefined}
      className={rowClass(isActive)} onClick={onClick}>{content}</button>;
  return siderTooltipProps ? <Tooltip {...siderTooltipProps} content={label} position='right'>{row}</Tooltip> : row;
}
