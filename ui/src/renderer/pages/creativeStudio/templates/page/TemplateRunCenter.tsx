/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Dropdown, Input, Menu, Message, Modal, Popover, Tooltip } from '@arco-design/web-react';
import {
  CheckOne,
  Close,
  Error,
  History,
  Loading,
  More,
  Pic,
  PreviewOpen,
  Refresh,
} from '@icon-park/react';
import React, { useEffect, useId, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { TFunction } from 'i18next';

import NomiInput from '@renderer/components/base/NomiInput';
import { ErrorModal, type ErrorFeedbackPresentation } from '@renderer/components/base/ErrorFeedback';
import { modalDefaults } from '@renderer/components/base/modalDefaults';
import { isHandledAuthExpiredHttpError } from '@/common/adapter/httpBridge';
import { getErrorTechnicalDetails } from '@/renderer/utils/ui/errorFeedback';
import { useCreativeAssetAvailability, type CreativeAsset } from '../../assets';
import CreativeMediaLightbox from '../../assets/components/CreativeMediaLightbox';

import type { CreativeTemplateRunAggregateV1 } from '../domain';
import type {
  CreativeTemplateRuntimeSnapshot,
  ReviewCreativeTemplateDraft,
} from '../runtime';
import dialogStyles from './TemplateWorkspaceDialogs.module.css';
import styles from './TemplateRunCenter.module.css';
import { formatTemplateLoadError } from '../templateI18n';

export interface CreativeTemplateRunCenterPort {
  snapshot: CreativeTemplateRuntimeSnapshot;
  assetUrl(assetId: string): string;
  assetReader?: { get(assetId: string): Promise<CreativeAsset> };
  resume(templateRunId: string): Promise<unknown>;
  cancel(templateRunId: string): Promise<unknown>;
  review(templateRunId: string, drafts: readonly ReviewCreativeTemplateDraft[]): Promise<unknown>;
  retry(run: CreativeTemplateRunAggregateV1): Promise<unknown>;
}

interface TemplateRunCenterProps {
  port: CreativeTemplateRunCenterPort;
  triggerLabel?: string;
  initialOpen?: boolean;
}

const ACTIVE = new Set(['requested', 'queued', 'running']);

function statusLabel(
  status: CreativeTemplateRunAggregateV1['record']['status'],
  t: TFunction
): string {
  const keyByStatus: Record<
    CreativeTemplateRunAggregateV1['record']['status'],
    string
  > = {
    requested: 'creativeStudio.templates.runCenter.status.requested',
    queued: 'creativeStudio.templates.runCenter.status.queued',
    running: 'creativeStudio.templates.runCenter.status.running',
    'awaiting-review': 'creativeStudio.templates.runCenter.status.awaitingReview',
    succeeded: 'creativeStudio.templates.runCenter.status.succeeded',
    failed: 'creativeStudio.templates.runCenter.status.failed',
    cancelled: 'creativeStudio.templates.runCenter.status.cancelled',
  };
  const defaultByStatus: Record<
    CreativeTemplateRunAggregateV1['record']['status'],
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

function RunStatusIcon({ run }: { run: CreativeTemplateRunAggregateV1 }) {
  const status = run.record.status;
  if (status === 'succeeded') return <CheckOne theme='outline' size={15} fill='currentColor' />;
  if (status === 'failed') return <Error theme='outline' size={15} fill='currentColor' />;
  if (status === 'cancelled') return <Close theme='outline' size={15} fill='currentColor' />;
  return <Loading theme='outline' size={15} fill='currentColor' />;
}

const TemplateRunReviewModal: React.FC<{
  run: CreativeTemplateRunAggregateV1 | null;
  saving: boolean;
  t: TFunction;
  onClose(): void;
  onApprove(drafts: readonly ReviewCreativeTemplateDraft[]): void;
}> = ({ run, saving, t, onClose, onApprove }) => {
  const [drafts, setDrafts] = useState<ReviewCreativeTemplateDraft[]>([]);

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

  const patch = (id: string, replacement: Partial<ReviewCreativeTemplateDraft>) => {
    setDrafts((current) => current.map((draft) =>
      draft.id === id ? { ...draft, ...replacement } : draft
    ));
  };

  return (
    <Modal
      visible
      className={dialogStyles.reviewModal}
      title={t('creativeStudio.templates.runCenter.review.title', {
        name: run.templateSnapshot.metadata.name,
        defaultValue: 'Review prompts · {{name}}',
      })}
      okText={t('creativeStudio.templates.runCenter.review.approve', {
        defaultValue: 'Approve and start generation',
      })}
      cancelText={t('creativeStudio.templates.runCenter.review.later', {
        defaultValue: 'Review later',
      })}
      confirmLoading={saving}
      okButtonProps={{ disabled: !valid }}
      autoFocus={false}
      unmountOnExit
      getPopupContainer={() => document.body}
      onCancel={onClose}
      onOk={() => {
        if (valid) onApprove(drafts);
      }}
    >
      <p className={dialogStyles.reviewHint}>
        {t('creativeStudio.templates.runCenter.review.hint', {
          defaultValue:
            "The plan has been persisted. Confirm each image title and prompt to continue within the template's concurrency limit.",
        })}
      </p>
      <div className={dialogStyles.reviewList}>
        {drafts.map((draft, index) => (
          <section key={draft.id} className={dialogStyles.reviewCard}>
            <div className={dialogStyles.reviewIndex}>{String(index + 1).padStart(2, '0')}</div>
            <div className={dialogStyles.reviewFields}>
              <NomiInput
                value={draft.title}
                disabled={saving}
                aria-label={t('creativeStudio.templates.runCenter.review.titleField', {
                  index: index + 1,
                  defaultValue: 'Image {{index}} title',
                })}
                placeholder={t('creativeStudio.templates.runCenter.review.titlePlaceholder', {
                  defaultValue: 'Image title',
                })}
                onChange={(title) => patch(draft.id, { title })}
              />
              <Input.TextArea
                value={draft.prompt}
                disabled={saving}
                aria-label={t('creativeStudio.templates.runCenter.review.promptField', {
                  index: index + 1,
                  defaultValue: 'Image {{index}} prompt',
                })}
                placeholder={t('creativeStudio.templates.runCenter.review.promptPlaceholder', {
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

const TemplateRunCenter: React.FC<TemplateRunCenterProps> = ({ port, triggerLabel, initialOpen = false }) => {
  const { t, i18n } = useTranslation();
  const locale = i18n.resolvedLanguage ?? i18n.language;
  const { snapshot } = port;
  const { runs } = snapshot;
  const [open, setOpen] = useState(initialOpen);
  const [reviewing, setReviewing] = useState<CreativeTemplateRunAggregateV1 | null>(null);
  const [actingId, setActingId] = useState<string | null>(null);
  const [preview, setPreview] = useState<{ runId: string; index: number } | null>(null);
  const [errorFeedback, setErrorFeedback] = useState<ErrorFeedbackPresentation | null>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const overlayTrigger = useRef<HTMLElement | null>(null);
  const popupId = useId();
  const titleId = useId();
  const title = t('creativeStudio.templates.runCenter.title', { defaultValue: 'Template runs' });
  const errorTitle = t('creativeStudio.templates.runCenter.errorTitle', { defaultValue: 'Template task error' });
  const availability = useCreativeAssetAvailability(runs.flatMap((run) => [
    ...run.record.resultAssetIds,
    ...run.request.referenceAssetIds,
    ...run.request.inputs.flatMap((input) => input.type === 'image'
      ? input.assetId ? [input.assetId] : []
      : input.type === 'image-series' ? input.assetIds : []),
  ]), port.assetReader);
  const unsettledCount = runs.filter((run) => ACTIVE.has(run.record.status)
    || run.record.status === 'awaiting-review'
    || snapshot.activities[run.request.id]?.state === 'paused').length;
  const unsettledLabel = t('creativeStudio.templates.runCenter.unsettledCount', {
    count: unsettledCount, defaultValue: '{{count}} unfinished',
  });
  const previewRun = preview ? runs.find((run) => run.request.id === preview.runId) : undefined;
  const previewIndex = Math.min(preview?.index ?? 0, Math.max(0, (previewRun?.record.resultAssetIds.length ?? 0) - 1));
  const previewAssetId = previewRun?.record.resultAssetIds[previewIndex];
  const previewAvailability = previewAssetId ? availability.get(previewAssetId) ?? 'loading' : 'loading';
  const overlayOpen = Boolean(reviewing || previewAssetId || errorFeedback);

  useEffect(() => {
    if (!reviewing) return;
    const replacement = runs.find((run) => run.request.id === reviewing.request.id);
    if (!replacement || replacement.record.status !== 'awaiting-review') setReviewing(null);
    else if (replacement.revision !== reviewing.revision) setReviewing(replacement);
  }, [reviewing, runs]);

  useEffect(() => {
    if (preview && !previewAssetId) setPreview(null);
  }, [preview, previewAssetId]);

  useEffect(() => {
    if (!overlayOpen) overlayTrigger.current?.focus({ preventScroll: true });
  }, [overlayOpen]);

  const rememberTrigger = (trigger?: HTMLElement) => {
    overlayTrigger.current = trigger ?? (document.activeElement instanceof HTMLElement ? document.activeElement : null);
  };
  const openResult = (run: CreativeTemplateRunAggregateV1, trigger?: HTMLElement) => {
    rememberTrigger(trigger);
    setPreview({ runId: run.request.id, index: 0 });
  };
  const openReview = (run: CreativeTemplateRunAggregateV1, trigger?: HTMLElement) => {
    rememberTrigger(trigger);
    setReviewing(run);
  };
  const closeList = () => {
    setOpen(false);
    triggerRef.current?.focus({ preventScroll: true });
  };
  const onListKeyDown = (event: React.KeyboardEvent) => {
    if (open && !overlayOpen && event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      closeList();
    }
  };
  const act = async (runId: string, action: () => Promise<unknown>, success?: string) => {
    if (actingId) return;
    setActingId(runId);
    try {
      await action();
      if (success) Message.success(success);
    } catch (error) {
      if (isHandledAuthExpiredHttpError(error)) return;
      rememberTrigger();
      setErrorFeedback({
        title: t('creativeStudio.templates.runCenter.actionError', { defaultValue: 'Template action failed' }),
        message: t('creativeStudio.templates.runCenter.actionErrorSummary', { defaultValue: 'The operation could not complete. Check the details and try again.' }),
        technicalDetails: getErrorTechnicalDetails(error),
        handled: false,
      });
    } finally {
      setActingId(null);
    }
  };

  const content = (
    <section id={popupId} className={styles.panel} data-template-run-list role='dialog' aria-modal={false} aria-labelledby={titleId} onKeyDown={onListKeyDown}>
      <header className={styles.header}>
        <History theme='outline' size={16} fill='currentColor' />
        <h2 id={titleId}>{title}</h2>
        <span className={styles.count}>{t('creativeStudio.templates.runCenter.count', { count: runs.length, defaultValue: '{{count}} runs' })}</span>
        <button type='button' className={styles.iconButton} aria-label={t('common.close')} onClick={closeList}>
          <Close theme='outline' size={14} fill='currentColor' />
        </button>
      </header>
      <div className={styles.list}>
        {snapshot.loadError ? <div className={styles.loadError} role='alert'>
          <Error theme='outline' size={16} fill='currentColor' />
          <span>{formatTemplateLoadError(snapshot.loadError, t)}</span>
        </div> : null}
        {runs.length === 0 && (!snapshot.loadError || snapshot.loading) ? <div className={styles.empty} role='status'>
          {snapshot.loading ? <>
            <Loading className={styles.spinning} theme='outline' size={22} fill='currentColor' />
            <span>{t('creativeStudio.templates.runCenter.loading', { defaultValue: 'Loading template runs…' })}</span>
          </> : !snapshot.loadError ? <>
            <History theme='outline' size={24} fill='currentColor' />
            <strong>{t('creativeStudio.templates.runCenter.empty', { defaultValue: 'No template runs yet' })}</strong>
            <span>{t('creativeStudio.templates.runCenter.emptyHint', { defaultValue: 'Run a template to see it here' })}</span>
          </> : null}
        </div> : null}
        {runs.map((run) => {
          const activity = snapshot.activities[run.request.id];
          const paused = activity?.state === 'paused';
          const active = ACTIVE.has(run.record.status);
          const total = run.record.taskIds.length;
          const completed = run.record.status === 'succeeded' ? total : run.record.taskIds.filter((id) => {
            const status = activity?.taskStatuses[id];
            return status === 'succeeded' || status === 'failed' || status === 'canceled';
          }).length;
          const progressLabel = t('creativeStudio.templates.runCenter.progress', {
            completed, total: total || t('creativeStudio.templates.runCenter.progressUnknown', { defaultValue: '-' }),
            defaultValue: 'Tasks {{completed}}/{{total}}',
          });
          const displayStatus = paused ? t('creativeStudio.templates.runCenter.status.paused', { defaultValue: 'Waiting to resume' }) : statusLabel(run.record.status, t);
          const technicalDetails = [...new Set([
            ...(run.record.failure ? [`code: ${run.record.failure.code}`, run.record.failure.message] : []),
            activity?.error,
          ].filter((detail): detail is string => Boolean(detail?.trim())))].join('\n\n');
          const inputIds = [...run.request.referenceAssetIds, ...run.request.inputs.flatMap((input) =>
            input.type === 'image' ? input.assetId ? [input.assetId] : [] : input.type === 'image-series' ? input.assetIds : [])];
          const deletedInputs = inputIds.some((id) => availability.get(id) === 'deleted');
          const resultCount = run.record.resultAssetIds.length;
          const canReview = run.record.status === 'awaiting-review';
          const canRetry = run.record.status === 'failed' || run.record.status === 'cancelled';
          const canOpen = canReview || resultCount > 0;
          const openRun = (trigger?: HTMLElement) => canReview ? openReview(run, trigger) : openResult(run, trigger);
          const statusContent = <><RunStatusIcon run={run} />{displayStatus}</>;
          return <article key={run.request.id} className={styles.row} data-run-id={run.request.id} data-run-status={run.record.status} data-clickable={canOpen}
            onClick={(event) => {
              if (canOpen && event.target instanceof Element && !event.target.closest('button, a, [role="menu"]')) {
                openRun(event.currentTarget.querySelector<HTMLButtonElement>('[data-open-run]') ?? undefined);
              }
            }}>
            <div className={styles.identity}>
              <Tooltip content={run.templateSnapshot.metadata.name}>
                {canOpen ? <button type='button' className={styles.name} data-open-run onClick={(event) => openRun(event.currentTarget)}>{run.templateSnapshot.metadata.name}</button>
                  : <span className={styles.name}>{run.templateSnapshot.metadata.name}</span>}
              </Tooltip>
              {run.record.status === 'failed' || technicalDetails ? <Tooltip content={t('creativeStudio.templates.runCenter.errorDetails', { defaultValue: 'View error details' })}>
                <button type='button' className={styles.status} data-status={paused ? 'paused' : run.record.status} aria-haspopup='dialog'
                  aria-label={`${displayStatus} · ${t('creativeStudio.templates.runCenter.errorDetails', { defaultValue: 'View error details' })}`}
                  onClick={(event) => {
                    rememberTrigger(event.currentTarget);
                    setErrorFeedback({
                      title: errorTitle,
                      message: technicalDetails
                        ? t('creativeStudio.templates.runCenter.errorSummary', { defaultValue: 'The template task could not complete. Check the details and try again.' })
                        : t('creativeStudio.templates.runCenter.errorUnknown', { defaultValue: 'No additional error details are available.' }),
                      technicalDetails,
                      handled: false,
                    });
                  }}>{statusContent}</button>
              </Tooltip> : <span className={styles.status} data-status={paused ? 'paused' : run.record.status}>{statusContent}</span>}
              {active || canReview || paused ? <Dropdown trigger='click' position='br' triggerProps={{ containerScrollToClose: true }}
                getPopupContainer={(node) => node.closest('[data-template-run-list]') ?? document.body}
                droplist={<Menu onClickMenuItem={() => void act(run.request.id, () => port.cancel(run.request.id), t('creativeStudio.templates.runCenter.cancelSuccess', { defaultValue: 'Template run cancelled' }))}>
                  <Menu.Item key='cancel' disabled={Boolean(actingId)}>{t('creativeStudio.templates.runCenter.cancel', { defaultValue: 'Cancel' })}</Menu.Item>
                </Menu>}>
                <button type='button' className={styles.iconButton} aria-label={t('creativeStudio.templates.runCenter.moreActions', { defaultValue: 'More task actions' })} disabled={Boolean(actingId)}>
                  <More theme='outline' size={16} fill='currentColor' />
                </button>
              </Dropdown> : null}
            </div>
            <div className={styles.meta}>
              <span className={styles.kind}><Pic theme='outline' size={12} fill='currentColor' />
                {run.templateSnapshot.output.kind === 'multi-image-series'
                  ? t('creativeStudio.templates.runCenter.kind.multi', { defaultValue: 'Image series' })
                  : t('creativeStudio.templates.runCenter.kind.single', { defaultValue: 'Single image' })}
              </span>
              <span aria-hidden>·</span><time dateTime={new Date(run.request.requestedAt).toISOString()}>{formatRunTime(run.request.requestedAt, locale)}</time>
              {resultCount > 1 || (resultCount > 0 && run.record.status !== 'succeeded') ? <button type='button' className={styles.results} onClick={(event) => openResult(run, event.currentTarget)}>
                {t('creativeStudio.templates.runCenter.results', { count: resultCount, defaultValue: '{{count}} images' })}
              </button> : null}
              <div className={styles.actions}>
                {paused ? <button type='button' disabled={Boolean(actingId)} aria-busy={actingId === run.request.id} onClick={() => void act(run.request.id, () => port.resume(run.request.id))}>
                  <Refresh theme='outline' size={13} fill='currentColor' />{t('creativeStudio.templates.runCenter.resume', { defaultValue: 'Resume' })}
                </button> : canReview ? <button type='button' className={styles.reviewAction} onClick={(event) => openReview(run, event.currentTarget)}>
                  {t('creativeStudio.templates.runCenter.reviewAction', { defaultValue: 'Review prompts' })}
                </button> : canRetry ? <Tooltip content={deletedInputs ? t('creativeStudio.assets.deletedReference', { defaultValue: '引用素材已删除，请重新选择后再生成。' }) : t('creativeStudio.templates.runCenter.retry', { defaultValue: 'Run again' })}>
                  <button type='button' disabled={deletedInputs || Boolean(actingId)} aria-busy={actingId === run.request.id}
                    onClick={() => void act(run.request.id, () => port.retry(run), t('creativeStudio.templates.runCenter.retrySuccess', { defaultValue: 'New run created' }))}>
                    <Refresh theme='outline' size={13} fill='currentColor' />{t('creativeStudio.templates.runCenter.retry', { defaultValue: 'Run again' })}
                  </button>
                </Tooltip> : run.record.status === 'succeeded' && resultCount > 0 ? <button type='button' onClick={(event) => openResult(run, event.currentTarget)}>
                  <PreviewOpen theme='outline' size={13} fill='currentColor' />{t('creativeStudio.templates.runCenter.openResult', { defaultValue: 'Open result' })}
                </button> : active ? <span>{progressLabel}</span> : null}
              </div>
            </div>
            {active && !paused && total > 0 ? <progress className={styles.progress} value={completed} max={total} aria-label={progressLabel} /> : null}
          </article>;
        })}
      </div>
      <footer className={styles.footer}>
        <span>{t('creativeStudio.templates.runCenter.saved', { defaultValue: 'Tasks and results are saved automatically' })}</span>
        <span>{t('creativeStudio.templates.runCenter.newestFirst', { defaultValue: 'Newest first' })}</span>
      </footer>
    </section>
  );

  return <div className={styles.entry} data-template-run-center data-labeled={Boolean(triggerLabel)}>
    <Popover trigger='click' position='br' popupVisible={open} unmountOnExit={false} escToClose={!overlayOpen} triggerProps={{ containerScrollToClose: true }}
      className={styles.popover} style={{ width: 360, maxWidth: 360 }} content={content}
      onVisibleChange={(visible) => { if (!overlayOpen) setOpen(visible); }} getPopupContainer={modalDefaults.getChildrenPopupContainer}>
      <button ref={triggerRef} type='button' className={styles.trigger} aria-label={triggerLabel ?? title} title={`${title} · ${unsettledLabel}`}
        aria-expanded={open} aria-haspopup='dialog' aria-controls={open ? popupId : undefined} onKeyDown={onListKeyDown}>
        <History theme='outline' size={17} fill='currentColor' />
        {triggerLabel ? <span>{triggerLabel}</span> : null}
        {unsettledCount > 0 ? <span className={styles.badge} aria-hidden>{unsettledCount > 99 ? '99+' : unsettledCount}</span> : null}
      </button>
    </Popover>
    {previewAssetId && previewRun ? <CreativeMediaLightbox kind='image'
      src={previewAvailability === 'available' ? port.assetUrl(previewAssetId) : ''}
      loading={previewAvailability === 'loading'}
      error={previewAvailability === 'deleted' ? t('creativeStudio.assets.deleted', { defaultValue: '素材已删除' })
        : previewAvailability === 'unavailable' ? t('creativeStudio.assets.library.mediaUnavailable', { defaultValue: '素材暂时无法预览' }) : null}
      title={t('creativeStudio.templates.runCenter.viewResult', { index: previewIndex + 1, defaultValue: 'View result {{index}}' })}
      fileName={`${previewRun.templateSnapshot.metadata.name}-result-${previewIndex + 1}.png`} mimeType='image/png'
      navigation={{ index: previewIndex, total: previewRun.record.resultAssetIds.length, onIndexChange: (index) => setPreview({ runId: previewRun.request.id, index }) }}
      onClose={() => setPreview(null)} /> : null}
    <TemplateRunReviewModal run={reviewing} saving={reviewing ? actingId === reviewing.request.id : false} t={t}
      onClose={() => !actingId && setReviewing(null)} onApprove={(drafts) => {
        if (reviewing) void act(reviewing.request.id, () => port.review(reviewing.request.id, drafts), t('creativeStudio.templates.runCenter.approveSuccess', { defaultValue: 'Prompts approved; generation started' }));
      }} />
    <ErrorModal error={errorFeedback} onClose={() => setErrorFeedback(null)} />
  </div>;
};

export default TemplateRunCenter;
