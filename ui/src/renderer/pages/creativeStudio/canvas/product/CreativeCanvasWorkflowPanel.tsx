/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  CheckOne,
  Error,
  Loading,
  Pic,
  Play,
  Refresh,
  Right,
  Workbench,
} from '@icon-park/react';
import classNames from 'classnames';
import React, { useMemo, useState } from 'react';

import type {
  WorkflowDefinitionV1,
  WorkflowRunAggregateV1,
} from '../../workflows/domain';
import type { CreativeWorkflowRuntimeSnapshot } from '../../workflows/runtime';
import styles from './CreativeCanvasWorkflowPanel.module.css';

export interface CreativeCanvasWorkflowPanelProps {
  workflows: readonly WorkflowDefinitionV1[];
  runtime: CreativeWorkflowRuntimeSnapshot;
  loading: boolean;
  error: string | null;
  disabled?: boolean;
  insertingRunId?: string | null;
  onRetry(): void;
  onRun(workflow: WorkflowDefinitionV1): void;
  onInsertResults(run: WorkflowRunAggregateV1): void;
  onOpenCenter(): void;
  className?: string;
}

const iconProps = {
  theme: 'outline' as const,
  size: 15,
  fill: 'currentColor',
  strokeWidth: 2.5,
};

const ACTIVE_STATUSES = new Set<WorkflowRunAggregateV1['record']['status']>([
  'requested',
  'queued',
  'running',
  'awaiting-review',
]);

const RUN_STATUS_LABELS: Record<WorkflowRunAggregateV1['record']['status'], string> = {
  requested: '准备中',
  queued: '已排队',
  running: '运行中',
  'awaiting-review': '待审核',
  succeeded: '已完成',
  failed: '失败',
  cancelled: '已取消',
};

function latestRunByWorkflow(
  runs: readonly WorkflowRunAggregateV1[]
): ReadonlyMap<string, WorkflowRunAggregateV1> {
  const latest = new Map<string, WorkflowRunAggregateV1>();
  for (const run of runs) {
    const workflowId = run.workflowSnapshot.id;
    const current = latest.get(workflowId);
    if (!current || run.request.requestedAt > current.request.requestedAt) {
      latest.set(workflowId, run);
    }
  }
  return latest;
}

function WorkflowRunStatus({ run }: { run: WorkflowRunAggregateV1 }) {
  const status = run.record.status;
  const icon = status === 'succeeded'
    ? <CheckOne {...iconProps} />
    : status === 'failed' || status === 'cancelled'
      ? <Error {...iconProps} />
      : <Loading className={styles.spinning} {...iconProps} />;
  return (
    <span className={styles.runStatus} data-status={status}>
      {icon}
      {RUN_STATUS_LABELS[status]}
    </span>
  );
}

/** Compact canvas projection over the canonical global workflow repository and run store. */
const CreativeCanvasWorkflowPanel: React.FC<CreativeCanvasWorkflowPanelProps> = ({
  workflows,
  runtime,
  loading,
  error,
  disabled = false,
  insertingRunId = null,
  onRetry,
  onRun,
  onInsertResults,
  onOpenCenter,
  className,
}) => {
  const [search, setSearch] = useState('');
  const runsByWorkflow = useMemo(() => latestRunByWorkflow(runtime.runs), [runtime.runs]);
  const filtered = useMemo(() => {
    const query = search.trim().toLocaleLowerCase();
    if (!query) return workflows;
    return workflows.filter((workflow) => [
      workflow.metadata.name,
      workflow.metadata.description,
      workflow.metadata.category,
      ...workflow.metadata.tags,
    ].join('\n').toLocaleLowerCase().includes(query));
  }, [search, workflows]);
  const activeCount = runtime.runs.filter((run) => ACTIVE_STATUSES.has(run.record.status)).length;

  return (
    <section
      className={classNames(styles.panel, className)}
      data-canvas-product-panel='workflows'
      aria-label='画布模板'
    >
      <header className={styles.header}>
        <div>
          <h2>模板</h2>
          <p>{workflows.length} 个模板{activeCount > 0 ? ` · ${activeCount} 个进行中` : ''}</p>
        </div>
        <button type='button' className={styles.iconButton} aria-label='打开模板工作台' onClick={onOpenCenter}>
          <Right {...iconProps} />
        </button>
      </header>

      <div className={styles.searchRow}>
        <input
          type='search'
          value={search}
          placeholder='搜索模板'
          aria-label='搜索模板'
          onChange={(event) => setSearch(event.currentTarget.value)}
        />
        <button
          type='button'
          className={styles.iconButton}
          aria-label='刷新模板'
          disabled={loading}
          onClick={onRetry}
        >
          <Refresh className={loading ? styles.spinning : undefined} {...iconProps} />
        </button>
      </div>

      {error || runtime.loadError ? (
        <div className={styles.error} role='alert'>
          <Error {...iconProps} />
          <span>{error ?? runtime.loadError}</span>
          <button type='button' onClick={onRetry}>重试</button>
        </div>
      ) : loading && workflows.length === 0 ? (
        <div className={styles.state} role='status'>
          <Loading className={styles.spinning} {...iconProps} />
          正在载入模板…
        </div>
      ) : filtered.length === 0 ? (
        <div className={styles.state} role='status'>
          <Workbench {...iconProps} />
          <strong>{workflows.length === 0 ? '暂无模板' : '没有匹配的模板'}</strong>
          <span>可前往模板工作台创建和配置模板。</span>
          <button type='button' onClick={onOpenCenter}>打开模板工作台</button>
        </div>
      ) : (
        <div className={styles.list} role='list' aria-label='可运行模板'>
          {filtered.map((workflow) => {
            const run = runsByWorkflow.get(workflow.id) ?? null;
            const active = run ? ACTIVE_STATUSES.has(run.record.status) : false;
            const resultCount = run?.record.resultAssetIds.length ?? 0;
            const inserting = run?.request.id === insertingRunId;
            return (
              <article key={workflow.id} className={styles.card} role='listitem'>
                <div className={styles.cardHeading}>
                  <div className={styles.workflowIcon}><Workbench {...iconProps} /></div>
                  <div className={styles.identity}>
                    <strong title={workflow.metadata.name}>{workflow.metadata.name}</strong>
                    <span>{workflow.metadata.category || '未分类'} · {workflow.output.kind === 'multi-image-series' ? `${workflow.output.targetCount} 张系列` : '单图'}</span>
                  </div>
                </div>
                {workflow.metadata.description ? (
                  <p className={styles.description}>{workflow.metadata.description}</p>
                ) : null}
                {run ? (
                  <div className={styles.runRow}>
                    <WorkflowRunStatus run={run} />
                    {resultCount > 0 ? <span>{resultCount} 项真实结果</span> : null}
                  </div>
                ) : null}
                <div className={styles.actions}>
                  <button
                    type='button'
                    disabled={disabled || active}
                    onClick={() => onRun(workflow)}
                  >
                    <Play {...iconProps} />
                    {active ? '正在运行' : run ? '再次运行' : '运行'}
                  </button>
                  {run && run.record.status === 'succeeded' && resultCount > 0 ? (
                    <button
                      type='button'
                      disabled={disabled || inserting}
                      onClick={() => onInsertResults(run)}
                    >
                      {inserting ? <Loading className={styles.spinning} {...iconProps} /> : <Pic {...iconProps} />}
                      {inserting ? '正在插入' : '插入结果'}
                    </button>
                  ) : run?.record.status === 'awaiting-review' ? (
                    <button type='button' onClick={onOpenCenter}>去审核</button>
                  ) : null}
                </div>
              </article>
            );
          })}
        </div>
      )}
    </section>
  );
};

export default CreativeCanvasWorkflowPanel;
