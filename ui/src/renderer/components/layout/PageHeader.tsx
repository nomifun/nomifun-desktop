import classNames from 'classnames';
import React from 'react';
import './PageHeader.css';

export const PAGE_TITLE_CLASS = 'app-page-title';

interface PageHeaderProps {
  title: React.ReactNode;
  description?: React.ReactNode;
  actions?: React.ReactNode;
  badge?: React.ReactNode;
  level?: 1 | 2;
  className?: string;
}

/** Shared heading and action layout for desktop destinations and workspaces. */
export default function PageHeader({ title, description, actions, badge, level = 1, className }: PageHeaderProps) {
  const Heading = level === 1 ? 'h1' : 'h2';
  return <header className={classNames('page-header', className)}>
    <div className='page-header__copy'>
      <div className='page-header__title-line'><Heading className={PAGE_TITLE_CLASS}>{title}</Heading>{badge}</div>
      {description && <div className='page-header__description'>{description}</div>}
    </div>
    {actions && <div className='page-header__actions'>{actions}</div>}
  </header>;
}
