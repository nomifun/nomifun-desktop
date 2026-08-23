/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Button, Input, Message, Modal } from '@arco-design/web-react';
import {
  CheckOne,
  Close,
  Download,
  Error,
  Loading,
  Refresh,
} from '@icon-park/react';
import React, { useEffect, useMemo, useState } from 'react';

import type { WorkflowRunAggregateV1 } from '../domain';
import type {
  CreativeWorkflowRuntimeSnapshot,
  ReviewCreativeWorkflowDraft,
} from '../runtime';
import styles from './CreativeWorkflowWorkspacePage.module.css';

export interface CreativeWorkflowRunCenterPort {
  snapshot: CreativeWorkflowRuntimeSnapshot;
  assetUrl(assetId: string): string;
  resume(runId: string): Promise<unknown>;
  cancel(runId: string): Promise<unknown>;
  review(runId: string, drafts: readonly ReviewCreativeWorkflowDraft[]): Promise<unknown>;
  retry(run: WorkflowRunAggregateV1): Promise<unknown>;
}

export interface WorkflowRunCenterProps {
  port: CreativeWorkflowRunCenterPort;
}

const ACTIVE = new Set(['requested', 'queued', 'running']);

const STATUS_LABELS: Record<WorkflowRunAggregateV1['record']['status'], string> = {
  requested: '准备中',
  queued: '已排队',
  running: '运行中',
  'awaiting-review': '待审核',
  succeeded: '已完成',
  failed: '失败',
  cancelled: '已取消',
};

function formatRunTime(timestamp: number): string {
  return new Intl.DateTimeFormat('zh-CN', {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  }).format(new Date(timestamp));
}

function errorMessage(error: unknown, fallback: string): string {
  return error instanceof globalThis.Error && error.message.trim() ? error.message : fallback;
}

function RunStatusIcon({ run }: { run: WorkflowRunAggregateV1 }) {
  const status = run.record.status;
  if (status === 'succeeded') return <CheckOne theme='outline' size={15} fill='currentColor' />;
  if (status === 'failed') return <Error theme='outline' size={15} fill='currentColor' />;
  if (status === 'cancelled') return <Close theme='outline' size={15} fill='currentColor' />;
  return <Loading theme='outline' size={15} fill='currentColor' />;
}

const WorkflowRunReviewModal: React.FC<{
  run: WorkflowRunAggregateV1 | null;
  saving: boolean;
  onClose(): void;
  onApprove(drafts: readonly ReviewCreativeWorkflowDraft[]): void;
}> = ({ run, saving, onClose, onApprove }) => {
  const [drafts, setDrafts] = useState<ReviewCreativeWorkflowDraft[]>([]);

  useEffect(() => {
    setDrafts(
      run?.promptDrafts.map((draft) => ({
        id: draft.id,
        title: draft.title,
        prompt: draft.prompt,
        reviewNote: draft.reviewNote,
      })) ?? []
    );
  }, [run?.request.id, run?.revision]);

  if (!run) return null;
  const valid = drafts.length === run.promptDrafts.length
    && drafts.every((draft) => draft.title.trim() && draft.prompt.trim());

  const patch = (id: string, replacement: Partial<ReviewCreativeWorkflowDraft>) => {
    setDrafts((current) => current.map((draft) =>
      draft.id === id ? { ...draft, ...replacement } : draft
    ));
  };

  return (
    <Modal
      visible
      className={styles.reviewModal}
      title={`审核提示词 · ${run.workflowSnapshot.metadata.name}`}
      okText='批准并开始生成'
      cancelText='稍后审核'
      confirmLoading={saving}
      okButtonProps={{ disabled: !valid }}
      autoFocus={false}
      unmountOnExit
      getPopupContainer={() =>
        document.getElementById('creative-studio-portal-root') ?? document.body
      }
      onCancel={onClose}
      onOk={() => {
        if (valid) onApprove(drafts);
      }}
    >
      <p className={styles.reviewHint}>
        规划结果已经持久化。确认每张图的标题和提示词后，任务会按模板并发上限继续执行。
      </p>
      <div className={styles.reviewList}>
        {drafts.map((draft, index) => (
          <section key={draft.id} className={styles.reviewCard}>
            <div className={styles.reviewIndex}>{String(index + 1).padStart(2, '0')}</div>
            <div className={styles.reviewFields}>
              <Input
                value={draft.title}
                disabled={saving}
                aria-label={`第 ${index + 1} 张标题`}
                placeholder='画面标题'
                onChange={(title) => patch(draft.id, { title })}
              />
              <Input.TextArea
                value={draft.prompt}
                disabled={saving}
                aria-label={`第 ${index + 1} 张提示词`}
                placeholder='图片提示词'
                autoSize={{ minRows: 3, maxRows: 7 }}
                onChange={(prompt) => patch(draft.id, { prompt })}
              />
            </div>
          </section>
        ))}
      </div>
    </Modal>
  );
};

const WorkflowRunCenter: React.FC<WorkflowRunCenterProps> = ({ port }) => {
  const { snapshot } = port;
  const [reviewing, setReviewing] = useState<WorkflowRunAggregateV1 | null>(null);
  const [actingId, setActingId] = useState<string | null>(null);
  const runs = snapshot.runs;
  const activeCount = runs.filter((run) => ACTIVE.has(run.record.status)).length;
  const reviewCount = runs.filter((run) => run.record.status === 'awaiting-review').length;

  useEffect(() => {
    if (!reviewing) return;
    const replacement = runs.find((run) => run.request.id === reviewing.request.id);
    if (!replacement || replacement.record.status !== 'awaiting-review') {
      setReviewing(null);
      return;
    }
    if (replacement.revision !== reviewing.revision) setReviewing(replacement);
  }, [reviewing, runs]);

  const cards = useMemo(() => runs.slice(0, 30), [runs]);
  if (cards.length === 0 && !snapshot.loadError) return null;

  const act = async (runId: string, action: () => Promise<unknown>, success?: string) => {
    if (actingId) return;
    setActingId(runId);
    try {
      await action();
      if (success) Message.success(success);
    } catch (error) {
      Message.error(errorMessage(error, '模板操作失败'));
    } finally {
      setActingId(null);
    }
  };

  return (
    <section className={styles.runCenter} data-workflow-run-center>
      <header className={styles.runCenterHeader}>
        <div className={styles.runCenterTitle}>
          <Loading
            className={activeCount > 0 ? styles.spinning : undefined}
            theme='outline'
            size={16}
            fill='currentColor'
          />
          <h2>模板任务</h2>
          <span className={styles.chip}>{runs.length} 个</span>
          {activeCount > 0 ? <span className={styles.runActiveChip}>{activeCount} 运行中</span> : null}
          {reviewCount > 0 ? <span className={styles.runReviewChip}>{reviewCount} 待审核</span> : null}
        </div>
        <p>任务、审核草稿和生成结果均由 NomiFun 持久化，刷新或重启后会继续恢复。</p>
      </header>

      {snapshot.loadError ? (
        <div className={styles.runLoadError} role='alert'>
          <Error theme='outline' size={16} fill='currentColor' />
          <span>运行记录加载失败：{snapshot.loadError}</span>
        </div>
      ) : null}

      <div className={styles.runCards}>
        {cards.map((run) => {
          const activity = snapshot.activities[run.request.id];
          const paused = activity?.state === 'paused';
          const observed = Object.values(activity?.taskStatuses ?? {});
          const completedTasks = observed.filter((status) =>
            status === 'succeeded' || status === 'failed' || status === 'canceled'
          ).length;
          const statusLabel = paused ? '等待恢复' : STATUS_LABELS[run.record.status];
          return (
            <article
              key={run.request.id}
              className={styles.runCard}
              data-run-id={run.request.id}
              data-run-status={run.record.status}
            >
              <div className={styles.runCardHeader}>
                <div className={styles.runCardIdentity}>
                  <h3>{run.workflowSnapshot.metadata.name}</h3>
                  <p>{formatRunTime(run.request.requestedAt)}</p>
                </div>
                <span
                  className={styles.runStatus}
                  data-status={paused ? 'paused' : run.record.status}
                >
                  <RunStatusIcon run={run} />
                  {statusLabel}
                </span>
              </div>

              <div className={styles.runProgressRow}>
                <span>{run.workflowSnapshot.output.kind === 'multi-image-series' ? '多图系列' : '单图任务'}</span>
                <span>
                  任务 {Math.max(completedTasks, run.record.status === 'succeeded' ? run.record.taskIds.length : 0)}
                  /{run.record.taskIds.length || '—'}
                </span>
              </div>

              {activity?.error ? <p className={styles.runError}>{activity.error}</p> : null}
              {run.record.failure ? <p className={styles.runError}>{run.record.failure.message}</p> : null}

              {run.record.resultAssetIds.length > 0 ? (
                <div className={styles.runResults}>
                  {run.record.resultAssetIds.slice(0, 6).map((assetId, index) => (
                    <a
                      key={assetId}
                      href={port.assetUrl(assetId)}
                      target='_blank'
                      rel='noreferrer'
                      title={`查看结果 ${index + 1}`}
                    >
                      <img src={port.assetUrl(assetId)} alt={`${run.workflowSnapshot.metadata.name} 结果 ${index + 1}`} />
                    </a>
                  ))}
                </div>
              ) : null}

              <div className={styles.runCardActions}>
                {paused ? (
                  <Button
                    size='small'
                    icon={<Refresh theme='outline' size={14} fill='currentColor' />}
                    loading={actingId === run.request.id}
                    onClick={() => void act(run.request.id, () => port.resume(run.request.id))}
                  >
                    继续
                  </Button>
                ) : null}
                {run.record.status === 'awaiting-review' ? (
                  <Button size='small' type='primary' onClick={() => setReviewing(run)}>
                    审核提示词
                  </Button>
                ) : null}
                {ACTIVE.has(run.record.status) || run.record.status === 'awaiting-review' ? (
                  <Button
                    size='small'
                    status='danger'
                    loading={actingId === run.request.id}
                    onClick={() => void act(
                      run.request.id,
                      () => port.cancel(run.request.id),
                      '模板任务已取消'
                    )}
                  >
                    取消
                  </Button>
                ) : null}
                {run.record.status === 'failed' || run.record.status === 'cancelled' ? (
                  <Button
                    size='small'
                    icon={<Refresh theme='outline' size={14} fill='currentColor' />}
                    loading={actingId === run.request.id}
                    onClick={() => void act(run.request.id, () => port.retry(run), '已创建新的运行')}
                  >
                    重新运行
                  </Button>
                ) : null}
                {run.record.status === 'succeeded' && run.record.resultAssetIds.length > 0 ? (
                  <Button
                    size='small'
                    icon={<Download theme='outline' size={14} fill='currentColor' />}
                    href={port.assetUrl(run.record.resultAssetIds[0])}
                    target='_blank'
                  >
                    打开结果
                  </Button>
                ) : null}
              </div>
            </article>
          );
        })}
      </div>

      <WorkflowRunReviewModal
        run={reviewing}
        saving={reviewing ? actingId === reviewing.request.id : false}
        onClose={() => !actingId && setReviewing(null)}
        onApprove={(drafts) => {
          if (!reviewing) return;
          void act(
            reviewing.request.id,
            () => port.review(reviewing.request.id, drafts),
            '提示词已批准，开始生成'
          );
        }}
      />
    </section>
  );
};

export default WorkflowRunCenter;
