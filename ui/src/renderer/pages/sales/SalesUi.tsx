import { Button } from '@arco-design/web-react';
import { Right } from '@icon-park/react';
import React from 'react';
import { Link } from 'react-router-dom';
import type { SalesCompanyStatus, SalesTaskStatus } from './salesWorkspace';

export const SalesPageHeader: React.FC<{
  title: string;
  description: string;
  action?: React.ReactNode;
}> = ({ title, description, action }) => (
  <header className='sales-page-header'>
    <div className='min-w-0'>
      <h1>{title}</h1>
      <p>{description}</p>
    </div>
    {action ? <div className='sales-page-header__action'>{action}</div> : null}
  </header>
);

export const SalesSection: React.FC<
  React.PropsWithChildren<{
    title?: string;
    description?: string;
    action?: React.ReactNode;
    className?: string;
  }>
> = ({ title, description, action, className = '', children }) => (
  <section className={`sales-section ${className}`.trim()}>
    {title || description || action ? (
      <div className='sales-section__header'>
        <div>
          {title ? <h2>{title}</h2> : null}
          {description ? <p>{description}</p> : null}
        </div>
        {action}
      </div>
    ) : null}
    {children}
  </section>
);

export const SalesEmptyState: React.FC<{
  icon: React.ReactNode;
  title: string;
  description: string;
  actionLabel?: string;
  actionTo?: string;
}> = ({ icon, title, description, actionLabel, actionTo }) => (
  <div className='sales-empty-state'>
    <span className='sales-empty-state__icon'>{icon}</span>
    <div>
      <h3>{title}</h3>
      <p>{description}</p>
    </div>
    {actionLabel && actionTo ? (
      <Link to={actionTo}>
        <Button type='primary'>
          {actionLabel} <Right size={14} />
        </Button>
      </Link>
    ) : null}
  </div>
);

const taskLabels: Record<SalesTaskStatus, string> = {
  draft: '计划已保存',
  queued: '等待执行',
  running: '执行中',
  paused: '已暂停',
  completed: '已完成',
  failed: '执行失败',
};

const companyLabels: Record<SalesCompanyStatus, string> = {
  sourcing: '正在寻找',
  researching: '正在分析',
  qualified: '符合条件',
  skipped: '已跳过',
  form_ready: '表单已准备',
  submitted: '已提交',
  failed: '需要处理',
};

export const SalesStatusPill: React.FC<{
  status: SalesTaskStatus | SalesCompanyStatus;
}> = ({ status }) => {
  const label = taskLabels[status as SalesTaskStatus] ?? companyLabels[status as SalesCompanyStatus] ?? status;
  return <span className={`sales-status sales-status--${status}`}>{label}</span>;
};

export const formatSalesDate = (iso: string) => {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return '—';
  return new Intl.DateTimeFormat('zh-CN', {
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  }).format(date);
};
