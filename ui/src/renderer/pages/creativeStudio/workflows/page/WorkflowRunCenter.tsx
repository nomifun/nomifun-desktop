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
import { useTranslation } from 'react-i18next';
import type { TFunction } from 'i18next';

import type { WorkflowRunAggregateV1 } from '../domain';
import type {
  CreativeWorkflowRuntimeSnapshot,
  ReviewCreativeWorkflowDraft,
} from '../runtime';
import styles from './CreativeWorkflowWorkspacePage.module.css';
import {
  formatWorkflowLoadError,
  formatWorkflowRuntimeError,
  workflowFallbackError,
} from '../workflowI18n';

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

function statusLabel(
  status: WorkflowRunAggregateV1['record']['status'],
  t: TFunction
): string {
  const keyByStatus: Record<
    WorkflowRunAggregateV1['record']['status'],
    string
  > = {
    requested: 'creativeStudio.workflows.runCenter.status.requested',
    queued: 'creativeStudio.workflows.runCenter.status.queued',
    running: 'creativeStudio.workflows.runCenter.status.running',
    'awaiting-review': 'creativeStudio.workflows.runCenter.status.awaitingReview',
    succeeded: 'creativeStudio.workflows.runCenter.status.succeeded',
    failed: 'creativeStudio.workflows.runCenter.status.failed',
    cancelled: 'creativeStudio.workflows.runCenter.status.cancelled',
  };
  const defaultByStatus: Record<
    WorkflowRunAggregateV1['record']['status'],
    string
  > = {
    requested: 'Preparing',
    queued: 'Queued',
    running: 'Running',
    'awaiting-review': 'Awaiting review',
    succeeded: 'Completed',
    failed: 'Failed',
    cancelled: 'Cancelled',
  };
  return t(keyByStatus[status], { defaultValue: defaultByStatus[status] });
}

function formatRunTime(timestamp: number, locale: string): string {
  return new Intl.DateTimeFormat(locale, {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  }).format(new Date(timestamp));
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
  t: TFunction;
  onClose(): void;
  onApprove(drafts: readonly ReviewCreativeWorkflowDraft[]): void;
}> = ({ run, saving, t, onClose, onApprove }) => {
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
      title={t('creativeStudio.workflows.runCenter.review.title', {
        name: run.workflowSnapshot.metadata.name,
        defaultValue: 'Review prompts · {{name}}',
      })}
      okText={t('creativeStudio.workflows.runCenter.review.approve', {
        defaultValue: 'Approve and start generation',
      })}
      cancelText={t('creativeStudio.workflows.runCenter.review.later', {
        defaultValue: 'Review later',
      })}
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
        {t('creativeStudio.workflows.runCenter.review.hint', {
          defaultValue:
            "The plan has been persisted. Confirm each image title and prompt to continue within the template's concurrency limit.",
        })}
      </p>
      <div className={styles.reviewList}>
        {drafts.map((draft, index) => (
          <section key={draft.id} className={styles.reviewCard}>
            <div className={styles.reviewIndex}>{String(index + 1).padStart(2, '0')}</div>
            <div className={styles.reviewFields}>
              <Input
                value={draft.title}
                disabled={saving}
                aria-label={t('creativeStudio.workflows.runCenter.review.titleField', {
                  index: index + 1,
                  defaultValue: 'Image {{index}} title',
                })}
                placeholder={t('creativeStudio.workflows.runCenter.review.titlePlaceholder', {
                  defaultValue: 'Image title',
                })}
                onChange={(title) => patch(draft.id, { title })}
              />
              <Input.TextArea
                value={draft.prompt}
                disabled={saving}
                aria-label={t('creativeStudio.workflows.runCenter.review.promptField', {
                  index: index + 1,
                  defaultValue: 'Image {{index}} prompt',
                })}
                placeholder={t('creativeStudio.workflows.runCenter.review.promptPlaceholder', {
                  defaultValue: 'Image prompt',
                })}
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
  const { t, i18n } = useTranslation();
  const locale = i18n.resolvedLanguage ?? i18n.language;
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
      Message.error(
        workflowFallbackError(
          error,
          t,
          'creativeStudio.workflows.runCenter.actionError',
          'Template action failed'
        )
      );
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
          <h2>
            {t('creativeStudio.workflows.runCenter.title', {
              defaultValue: 'Template runs',
            })}
          </h2>
          <span className={styles.chip}>
            {t('creativeStudio.workflows.runCenter.count', {
              count: runs.length,
              defaultValue: '{{count}} runs',
            })}
          </span>
          {activeCount > 0 ? (
            <span className={styles.runActiveChip}>
              {t('creativeStudio.workflows.runCenter.activeCount', {
                count: activeCount,
                defaultValue: '{{count}} running',
              })}
            </span>
          ) : null}
          {reviewCount > 0 ? (
            <span className={styles.runReviewChip}>
              {t('creativeStudio.workflows.runCenter.reviewCount', {
                count: reviewCount,
                defaultValue: '{{count}} awaiting review',
              })}
            </span>
          ) : null}
        </div>
        <p>
          {t('creativeStudio.workflows.runCenter.description', {
            defaultValue:
              'NomiFun persists tasks, review drafts, and generated results. They will be restored after a refresh or restart.',
          })}
        </p>
      </header>

      {snapshot.loadError ? (
        <div className={styles.runLoadError} role='alert'>
          <Error theme='outline' size={16} fill='currentColor' />
          <span>{formatWorkflowLoadError(snapshot.loadError, t)}</span>
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
          const displayStatus = paused
            ? t('creativeStudio.workflows.runCenter.status.paused', {
                defaultValue: 'Waiting to resume',
              })
            : statusLabel(run.record.status, t);
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
                  <p>{formatRunTime(run.request.requestedAt, locale)}</p>
                </div>
                <span
                  className={styles.runStatus}
                  data-status={paused ? 'paused' : run.record.status}
                >
                  <RunStatusIcon run={run} />
                  {displayStatus}
                </span>
              </div>

              <div className={styles.runProgressRow}>
                <span>
                  {run.workflowSnapshot.output.kind === 'multi-image-series'
                    ? t('creativeStudio.workflows.runCenter.kind.multi', {
                        defaultValue: 'Multi-image series',
                      })
                    : t('creativeStudio.workflows.runCenter.kind.single', {
                        defaultValue: 'Single-image task',
                      })}
                </span>
                <span>
                  {t('creativeStudio.workflows.runCenter.progress', {
                    completed: Math.max(
                      completedTasks,
                      run.record.status === 'succeeded' ? run.record.taskIds.length : 0
                    ),
                    total:
                      run.record.taskIds.length ||
                      t('creativeStudio.workflows.runCenter.progressUnknown', {
                        defaultValue: '-',
                      }),
                    defaultValue: 'Tasks {{completed}}/{{total}}',
                  })}
                </span>
              </div>

              {activity?.error ? (
                <p className={styles.runError}>
                  {formatWorkflowRuntimeError(activity.error, t)}
                </p>
              ) : null}
              {run.record.failure ? (
                <p className={styles.runError}>
                  {formatWorkflowRuntimeError(
                    run.record.failure.message,
                    t,
                    run.record.failure.code
                  )}
                </p>
              ) : null}

              {run.record.resultAssetIds.length > 0 ? (
                <div className={styles.runResults}>
                  {run.record.resultAssetIds.slice(0, 6).map((assetId, index) => (
                    <a
                      key={assetId}
                      href={port.assetUrl(assetId)}
                      target='_blank'
                      rel='noreferrer'
                      title={t('creativeStudio.workflows.runCenter.viewResult', {
                        index: index + 1,
                        defaultValue: 'View result {{index}}',
                      })}
                    >
                      <img
                        src={port.assetUrl(assetId)}
                        alt={t('creativeStudio.workflows.runCenter.resultAlt', {
                          name: run.workflowSnapshot.metadata.name,
                          index: index + 1,
                          defaultValue: '{{name}} result {{index}}',
                        })}
                      />
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
                    {t('creativeStudio.workflows.runCenter.resume', {
                      defaultValue: 'Resume',
                    })}
                  </Button>
                ) : null}
                {run.record.status === 'awaiting-review' ? (
                  <Button size='small' type='primary' onClick={() => setReviewing(run)}>
                    {t('creativeStudio.workflows.runCenter.reviewAction', {
                      defaultValue: 'Review prompts',
                    })}
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
                      t('creativeStudio.workflows.runCenter.cancelSuccess', {
                        defaultValue: 'Template run cancelled',
                      })
                    )}
                  >
                    {t('creativeStudio.workflows.runCenter.cancel', {
                      defaultValue: 'Cancel',
                    })}
                  </Button>
                ) : null}
                {run.record.status === 'failed' || run.record.status === 'cancelled' ? (
                  <Button
                    size='small'
                    icon={<Refresh theme='outline' size={14} fill='currentColor' />}
                    loading={actingId === run.request.id}
                    onClick={() =>
                      void act(
                        run.request.id,
                        () => port.retry(run),
                        t('creativeStudio.workflows.runCenter.retrySuccess', {
                          defaultValue: 'New run created',
                        })
                      )
                    }
                  >
                    {t('creativeStudio.workflows.runCenter.retry', {
                      defaultValue: 'Run again',
                    })}
                  </Button>
                ) : null}
                {run.record.status === 'succeeded' && run.record.resultAssetIds.length > 0 ? (
                  <Button
                    size='small'
                    icon={<Download theme='outline' size={14} fill='currentColor' />}
                    href={port.assetUrl(run.record.resultAssetIds[0])}
                    target='_blank'
                  >
                    {t('creativeStudio.workflows.runCenter.openResult', {
                      defaultValue: 'Open result',
                    })}
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
        t={t}
        onClose={() => !actingId && setReviewing(null)}
        onApprove={(drafts) => {
          if (!reviewing) return;
          void act(
            reviewing.request.id,
            () => port.review(reviewing.request.id, drafts),
            t('creativeStudio.workflows.runCenter.approveSuccess', {
              defaultValue: 'Prompts approved; generation started',
            })
          );
        }}
      />
    </section>
  );
};

export default WorkflowRunCenter;
