/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Spin } from '@arco-design/web-react';
import React, { useEffect, useMemo, useRef } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';

import { CREATIVE_STUDIO_PROJECTS_PATH } from '../../app/routes';
import type { CreativeProjectDetail, CreativeProjectSummary } from '../../domain';
import {
  isCreativeProjectRepositoryError,
  useCreativeProject,
  useCreativeProjects,
} from '../../services';
import { parseStandaloneProjectQuery, standaloneProjectSearch } from './ownership';
import styles from './StandaloneWorkbenchProduct.module.css';

export type StandaloneWorkbenchScopeState =
  | 'missing'
  | 'invalid'
  | 'loading'
  | 'not-found'
  | 'error'
  | 'ready';

export interface StandaloneWorkbenchScope {
  state: StandaloneWorkbenchScopeState;
  projectId: string | null;
  detail: CreativeProjectDetail | undefined;
  projects: CreativeProjectSummary[];
  projectsLoading: boolean;
  message: string;
  selectProject(projectId: string | null): void;
  openProjectCenter(): void;
  refreshProject(): Promise<CreativeProjectDetail | undefined>;
}

export function useStandaloneWorkbenchScope(): StandaloneWorkbenchScope {
  const location = useLocation();
  const navigate = useNavigate();
  const query = useMemo(() => parseStandaloneProjectQuery(location.search), [location.search]);
  const list = useCreativeProjects();
  const projectId = query.state === 'valid' ? query.projectId : null;
  const project = useCreativeProject(projectId);

  let state: StandaloneWorkbenchScopeState;
  let message: string;
  if (query.state === 'missing') {
    state = 'missing';
    message = '请选择一个真实项目作为任务归属；不会自动借用最近项目。';
  } else if (query.state === 'invalid') {
    state = 'invalid';
    message = query.message;
  } else if (project.isLoading && !project.detail) {
    state = 'loading';
    message = '正在验证项目归属…';
  } else if (project.error) {
    state =
      isCreativeProjectRepositoryError(project.error) && project.error.kind === 'not-found'
        ? 'not-found'
        : 'error';
    message = state === 'not-found' ? '指定项目不存在或已被删除。' : project.error.message;
  } else if (project.detail?.project.projectId === projectId) {
    state = 'ready';
    message = `任务与结果将写入项目「${project.detail.project.title}」的独立工作台历史。`;
  } else {
    state = 'loading';
    message = '正在验证项目归属…';
  }

  return {
    state,
    projectId,
    detail: project.detail,
    projects: list.projects,
    projectsLoading: list.isLoading,
    message,
    selectProject: (nextProjectId) => {
      navigate(
        {
          pathname: location.pathname,
          search: standaloneProjectSearch(location.search, nextProjectId),
        },
        { replace: true }
      );
    },
    openProjectCenter: () => navigate(CREATIVE_STUDIO_PROJECTS_PATH),
    refreshProject: project.refresh,
  };
}

export const StandaloneScopeBar: React.FC<{ scope: StandaloneWorkbenchScope }> = ({ scope }) => {
  const tone = scope.state === 'ready' ? 'neutral' : scope.state === 'error' ? 'danger' : 'warning';
  const noProjects = !scope.projectsLoading && scope.projects.length === 0;
  return (
    <div className={styles.scopeBar} data-standalone-scope={scope.state} data-tone={tone}>
      <div className={styles.scopeCopy}>
        <strong>{scope.state === 'ready' ? '项目归属已验证' : '需要明确项目归属'}</strong>
        <span>{noProjects ? '当前没有项目，请先在项目中心创建。' : scope.message}</span>
      </div>
      {scope.projectsLoading ? (
        <Spin size={18} />
      ) : (
        <select
          className={styles.scopeSelect}
          value={scope.projectId ?? ''}
          disabled={noProjects}
          aria-label='独立工作台项目归属'
          onChange={(event) => scope.selectProject(event.target.value || null)}
        >
          <option value=''>选择项目…</option>
          {scope.projects.map((project) => (
            <option key={project.projectId} value={project.projectId}>
              {project.title}
            </option>
          ))}
        </select>
      )}
      <button type='button' className={styles.scopeAction} onClick={scope.openProjectCenter}>
        {noProjects ? '创建项目' : '项目中心'}
      </button>
    </div>
  );
};

export const StandaloneWorkbenchPage: React.FC<{
  scope: StandaloneWorkbenchScope;
  error: string | null;
  children: React.ReactNode;
}> = ({ scope, error, children }) => (
  <div className={styles.page} data-standalone-workbench-page>
    <StandaloneScopeBar scope={scope} />
    {error ? (
      <div className={styles.runtimeNotice} role='alert'>
        {error}
      </div>
    ) : null}
    <div className={styles.workbench}>{children}</div>
  </div>
);

export const StandaloneHistoryGate: React.FC<{
  label: string;
  error: Error | null;
  onRetry(): void;
}> = ({ label, error, onRetry }) => (
  <section className={styles.historyGate} role={error ? 'alert' : 'status'}>
    <strong>{error ? `${label}历史加载失败` : `正在恢复${label}历史`}</strong>
    <p>{error?.message ?? '正在读取当前项目的真实任务与结果，请稍候。'}</p>
    {error ? (
      <button type='button' onClick={onRetry}>
        重试
      </button>
    ) : null}
  </section>
);

export const StandaloneHistoryRetireDialog: React.FC<{
  open: boolean;
  count: number;
  busy: boolean;
  error?: string | null;
  onCancel(): void;
  onConfirm(): void;
}> = ({ open, count, busy, error, onCancel, onConfirm }) => {
  const dialogRef = useRef<HTMLElement | null>(null);
  useEffect(() => {
    if (open) dialogRef.current?.focus();
  }, [open]);
  if (!open) return null;
  return (
    <div
      className={styles.retireBackdrop}
      onMouseDown={(event) => {
        if (!busy && event.target === event.currentTarget) onCancel();
      }}
    >
      <section
        ref={dialogRef}
        className={styles.retireDialog}
        role='dialog'
        aria-modal='true'
        aria-labelledby='standalone-retire-title'
        tabIndex={-1}
        onKeyDown={(event) => {
          if (event.key === 'Escape' && !busy) onCancel();
        }}
      >
        <h2 id='standalone-retire-title'>从历史移除{count > 1 ? ` ${count} 条` : '这条记录'}？</h2>
        <p>
          任务审计、输入素材和生成结果会继续安全保留；这里只让所选记录不再出现在当前工作台历史中。
        </p>
        {error ? (
          <p className={styles.retireError} role='alert'>
            {error}
          </p>
        ) : null}
        <div className={styles.retireActions}>
          <button type='button' disabled={busy} onClick={onCancel}>
            取消
          </button>
          <button type='button' data-danger disabled={busy} onClick={onConfirm}>
            {busy ? '正在移除…' : '从历史移除'}
          </button>
        </div>
      </section>
    </div>
  );
};
