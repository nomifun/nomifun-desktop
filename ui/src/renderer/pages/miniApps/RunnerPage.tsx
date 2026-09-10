/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import type {
  MiniAppOperationState,
  MiniAppPublishMode,
  MiniAppReleaseRef,
  MiniAppServiceLifecycle,
  MiniAppSurfaceLaunchDescriptor,
  MiniAppWorkshop,
} from '@/common/types/miniAppPlatform';
import { parseMiniAppId } from '@/common/types/ids';
import HubPageShell from '@/renderer/components/layout/HubPageShell';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import { Button, Modal, Radio, Tooltip } from '@arco-design/web-react';
import {
  ArrowLeft,
  CheckOne,
  CloseOne,
  Code,
  Delete,
  Download,
  Edit,
  Power,
  PreviewOpen,
  Refresh,
  ShareOne,
  Undo,
  Upload,
} from '@icon-park/react';
import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate, useParams } from 'react-router-dom';
import {
  formatMiniAppTimestamp,
  miniAppBuildRequest,
  miniAppCanOpenSurface,
  miniAppDeleteRequest,
  miniAppPublishRequest,
  miniAppRestoreRequest,
  miniAppRetryDeleteRequest,
  miniAppRetryServiceRequest,
  miniAppRollbackRequest,
  miniAppSetEnabledRequest,
  miniAppSetServiceRunningRequest,
  miniAppTrashRequest,
  miniAppSetPublishModeRequest,
  miniAppTestRequest,
  miniAppSurfaceAssetPath,
  miniAppSurfaceMatchesWorkshop,
  miniAppWorkflowState,
  shortMiniAppIdentity,
} from './model';
import MiniAppSurfacePanel from './MiniAppSurfacePanel';
import MiniAppSourceEditDialog from './MiniAppSourceEditDialog';
import MiniAppTransferDialog from './MiniAppTransferDialog';
import {
  MiniAppKindBadge,
  MiniAppHealthBadge,
  MiniAppLifecycleBadge,
  MiniAppSourceBadge,
  MiniAppStatePanel,
  MiniAppTestBadge,
  StatusBadge,
  WorkflowIcon,
} from './MiniAppM1State';
import styles from './MiniAppWorkbench.module.css';

type MiniAppBusyAction =
  | 'publish'
  | 'rollback'
  | 'enable'
  | 'disable'
  | 'publish_mode'
  | 'service_start'
  | 'service_stop'
  | 'service_retry'
  | 'test'
  | 'trash'
  | 'restore'
  | 'delete'
  | 'retry_delete'
  | 'open_surface'
  | 'reload_surface'
  | 'close_surface'
  | null;

function formatError(error: unknown): string {
  if (isBackendHttpError(error)) {
    const detail = error.backendMessage || error.message;
    return error.code ? `${error.code}: ${detail}` : detail;
  }
  return error instanceof Error ? error.message : String(error);
}

function isPermanentDeleteRunning(
  workshop: MiniAppWorkshop | null | undefined
): boolean {
  return Boolean(
    workshop?.miniapp.lifecycle === 'deleting' &&
      workshop.active_operation?.kind === 'miniapp_permanent_delete' &&
      workshop.active_operation.state === 'running'
  );
}

const OperationBadge: React.FC<{ state: MiniAppOperationState }> = ({
  state,
}) => {
  const { t } = useTranslation();
  const labels: Record<MiniAppOperationState, string> = {
    running: t('miniApps.workshop.operation.state.running'),
    succeeded: t('miniApps.workshop.operation.state.succeeded'),
    failed: t('miniApps.workshop.operation.state.failed'),
    canceled: t('miniApps.workshop.operation.state.canceled'),
  };
  const tones: Record<
    MiniAppOperationState,
    'success' | 'info' | 'danger' | 'muted'
  > = {
    running: 'info',
    succeeded: 'success',
    failed: 'danger',
    canceled: 'muted',
  };
  return <StatusBadge label={labels[state]} tone={tones[state]} />;
};

const Fact: React.FC<{
  label: string;
  value?: React.ReactNode;
  title?: string;
  mono?: boolean;
}> = ({ label, value, title, mono = false }) => (
  <div className={styles.fact}>
    <span className={styles.factLabel}>{label}</span>
    <span
      className={`${styles.factValue} ${mono ? styles.mono : ''}`}
      title={title}
    >
      {value ?? '—'}
    </span>
  </div>
);

const ReadyReleaseSummary: React.FC<{
  release: MiniAppReleaseRef;
  buildGeneration: number;
  createdAtMs: number;
}> = ({ release, buildGeneration, createdAtMs }) => {
  const { t, i18n } = useTranslation();
  return (
    <div className={styles.releaseGrid}>
      <div className={`${styles.releaseItem} ${styles.releaseItemReady}`}>
        <div className={styles.releaseName}>
          {t('miniApps.workshop.releases.ready')}
        </div>
        <div className={styles.releaseValue} title={release.release_id}>
          {shortMiniAppIdentity(release.release_id)}
        </div>
        <div className={styles.releaseDigest} title={release.release_digest}>
          {shortMiniAppIdentity(release.release_digest, 12)}
        </div>
      </div>
      <div className={styles.releaseItem}>
        <div className={styles.releaseName}>
          {t('miniApps.workshop.ready.artifact')}
        </div>
        <div className={styles.releaseValue} title={release.artifact_id}>
          {shortMiniAppIdentity(release.artifact_id)}
        </div>
        <div className={styles.releaseDigest} title={release.manifest_digest}>
          {t('miniApps.workshop.ready.manifest')}{' '}
          {shortMiniAppIdentity(release.manifest_digest, 8)}
        </div>
      </div>
      <div className={styles.releaseItem}>
        <div className={styles.releaseName}>
          {t('miniApps.workshop.ready.createdAt')}
        </div>
        <div className={styles.releaseValue}>
          {formatMiniAppTimestamp(createdAtMs, i18n.language)}
        </div>
        <div className={styles.releaseDigest}>
          {t('miniApps.workshop.ready.buildGeneration')} {buildGeneration}
        </div>
      </div>
    </div>
  );
};

export const MiniAppWorkshopDetail: React.FC<{
  workshop: MiniAppWorkshop;
  locale: string;
  onBack: () => void;
  onRefresh: () => void;
  onEditSource: () => void;
  onBuild: () => void;
  onTest: () => void;
  onCancelBuild: () => void;
  onPublish: () => void;
  onRollback: () => void;
  onSetEnabled: (enabled: boolean) => void;
  onSetPublishMode: (mode: MiniAppPublishMode) => void;
  onSetServiceLifecycle: (lifecycle: MiniAppServiceLifecycle) => void;
  onSetServiceRunning: (running: boolean) => void;
  onRetryService: () => void;
  onTrash: () => void;
  onRestore: () => void;
  onDelete: () => void;
  onRetryDelete: () => void;
  onShare: () => void;
  onBackup: () => void;
  onOpenSurface: () => void;
  onReloadSurface: () => void;
  onCloseSurface: () => void;
  surfaceDescriptor: MiniAppSurfaceLaunchDescriptor | null;
  busyAction: MiniAppBusyAction;
  refreshing: boolean;
  building: boolean;
  canceling: boolean;
  serviceLifecycle: MiniAppServiceLifecycle;
}> = ({
  workshop,
  locale,
  onBack,
  onRefresh,
  onEditSource,
  onBuild,
  onTest,
  onCancelBuild,
  onPublish,
  onRollback,
  onSetEnabled,
  onSetPublishMode,
  onSetServiceLifecycle,
  onSetServiceRunning,
  onRetryService,
  onTrash,
  onRestore,
  onDelete,
  onRetryDelete,
  onShare,
  onBackup,
  onOpenSurface,
  onReloadSurface,
  onCloseSurface,
  surfaceDescriptor,
  busyAction,
  refreshing,
  building,
  canceling,
  serviceLifecycle,
}) => {
  const { t } = useTranslation();
  const { miniapp, ready, active_operation: operation } = workshop;
  const activeService = workshop.active_service ?? ready?.service;
  const workflow = miniAppWorkflowState(workshop);
  const buildRunning =
    operation?.kind === 'build' && operation.state === 'running';
  const lifecycleActive =
    miniapp.lifecycle === 'enabled' || miniapp.lifecycle === 'disabled';
  const permanentDeleteRunning = isPermanentDeleteRunning(workshop);
  const permanentDeleteFailed =
    miniapp.lifecycle === 'deleting' &&
    operation?.kind === 'miniapp_permanent_delete' &&
    operation.state === 'failed';
  const canBuild =
    lifecycleActive &&
    miniAppBuildRequest(workshop, serviceLifecycle) !== null;
  const canEditSource =
    lifecycleActive &&
    workshop.source_state === 'editable' &&
    operation?.state !== 'running';
  const canTest = miniAppTestRequest(workshop) !== null;
  const canPublish =
    lifecycleActive && miniAppPublishRequest(workshop) !== null;
  const canRollback =
    lifecycleActive && miniAppRollbackRequest(workshop) !== null;
  const canEnable =
    lifecycleActive && miniAppSetEnabledRequest(workshop, true) !== null;
  const canDisable =
    lifecycleActive && miniAppSetEnabledRequest(workshop, false) !== null;
  const canOpenSurface =
    lifecycleActive && miniAppCanOpenSurface(workshop);
  const canTrash = miniAppTrashRequest(workshop) !== null;
  const canRestore = miniAppRestoreRequest(workshop) !== null;
  const canDelete = miniAppDeleteRequest(workshop) !== null;
  const canRetryDelete = miniAppRetryDeleteRequest(workshop) !== null;
  const canShare = Boolean(
    lifecycleActive &&
      (workshop.ready || miniapp.releases.active) &&
      operation?.state !== 'running'
  );
  const autoPublishAvailable = Boolean(
    miniapp.kind === 'ui_only' &&
      miniapp.releases.active &&
      (miniapp.lifecycle === 'enabled' || miniapp.lifecycle === 'disabled')
  );
  const idPrefix = `miniapp-workshop-${miniapp.miniapp_id}`;
  const detailTitleId = `${idPrefix}-title`;
  const detailDescriptionId = `${idPrefix}-description`;
  const sourceBuildTitleId = `${idPrefix}-source-build-title`;
  const sourceBuildHintId = `${idPrefix}-source-build-hint`;
  const releasesTitleId = `${idPrefix}-releases-title`;
  const releasesHintId = `${idPrefix}-releases-hint`;
  const serviceTitleId = `${idPrefix}-service-title`;
  const serviceHintId = `${idPrefix}-service-hint`;
  const publishModeTitleId = `${idPrefix}-publish-mode-title`;
  const publishModeHintId = `${idPrefix}-publish-mode-hint`;
  const readyTitleId = `${idPrefix}-ready-title`;
  const readyHintId = `${idPrefix}-ready-hint`;
  const operationTitleId = `${idPrefix}-operation-title`;
  const operationHintId = `${idPrefix}-operation-hint`;
  const namedAction = (label: string): string =>
    `${label}: ${miniapp.display_name}`;
  const primaryAction = canPublish
    ? 'publish'
    : canEnable
      ? 'enable'
      : canOpenSurface
        ? 'surface'
        : canBuild
          ? 'build'
          : null;
  const controlsDisabled = busyAction !== null || building || canceling;
  const workflowSteps = [
    {
      key: 'source',
      label: t('miniApps.workshop.workflow.source'),
      state: workflow.source,
    },
    {
      key: 'build',
      label: t('miniApps.workshop.workflow.build'),
      state: workflow.build,
    },
    {
      key: 'ready',
      label: t('miniApps.workshop.workflow.ready'),
      state: workflow.ready,
    },
    {
      key: 'publish',
      label: t('miniApps.workshop.workflow.publish'),
      state: workflow.publish,
    },
    {
      key: 'surface',
      label: t('miniApps.workshop.workflow.surface'),
      state: workflow.surface,
    },
  ] as const;

  return (
    <main
      className={styles.detail}
      aria-labelledby={detailTitleId}
      aria-describedby={detailDescriptionId}
      aria-busy={controlsDisabled || undefined}
    >
      <header className={styles.detailHeader}>
        <div className={styles.detailHeaderCopy}>
          <span className={styles.eyebrow}>
            {t('miniApps.workshop.detailEyebrow')}
          </span>
          <div className={styles.detailTitleRow}>
            <h2 id={detailTitleId} className={styles.detailTitle}>
              {miniapp.display_name}
            </h2>
            <MiniAppKindBadge kind={miniapp.kind} />
            <MiniAppLifecycleBadge lifecycle={miniapp.lifecycle} />
            <MiniAppSourceBadge source={workshop.source_state} />
          </div>
          <p id={detailDescriptionId} className={styles.detailDescription}>
            {miniapp.description || t('miniApps.library.noDescription')}
          </p>
        </div>
      </header>

      <div
        className={styles.actionBar}
        role='toolbar'
        aria-label={namedAction(t('miniApps.workshop.detailEyebrow'))}
      >
        <Button
          icon={<ArrowLeft theme='outline' size='14' />}
          aria-label={t('miniApps.actions.backToLibrary')}
          onClick={onBack}
        >
          {t('miniApps.actions.backToLibrary')}
        </Button>
        {lifecycleActive && (
          <>
            <Button
              icon={<Edit theme='outline' size='14' />}
              aria-label={namedAction(t('miniApps.actions.editSource'))}
              disabled={!canEditSource || controlsDisabled}
              onClick={onEditSource}
            >
              {t('miniApps.actions.editSource')}
            </Button>
            <Button
              type={primaryAction === 'build' ? 'primary' : 'default'}
              icon={<Code theme='outline' size='14' />}
              aria-label={namedAction(t('miniApps.actions.build'))}
              loading={building}
              disabled={!canBuild || controlsDisabled}
              onClick={onBuild}
            >
              {t('miniApps.actions.build')}
            </Button>
            {miniapp.kind === 'service' && (
              <Button
                icon={<CheckOne theme='outline' size='14' />}
                aria-label={namedAction(t('miniApps.actions.testService'))}
                loading={busyAction === 'test'}
                disabled={controlsDisabled || !canTest}
                onClick={onTest}
              >
                {t('miniApps.actions.testService')}
              </Button>
            )}
            <Tooltip
              content={
                canPublish
                  ? ''
                  : ready
                    ? t('miniApps.errors.publishUnavailable')
                    : t('miniApps.workshop.ready.emptyBody')
              }
              disabled={canPublish}
            >
              <span>
                <Button
                  type={primaryAction === 'publish' ? 'primary' : 'default'}
                  icon={<Upload theme='outline' size='14' />}
                  aria-label={namedAction(t('miniApps.actions.publish'))}
                  title={
                    canPublish
                      ? undefined
                      : ready
                        ? t('miniApps.errors.publishUnavailable')
                        : t('miniApps.workshop.ready.emptyBody')
                  }
                  loading={busyAction === 'publish'}
                  disabled={controlsDisabled || !canPublish}
                  onClick={onPublish}
                >
                  {t('miniApps.actions.publish')}
                </Button>
              </span>
            </Tooltip>
            <Tooltip
              content={
                canRollback
                  ? ''
                  : t('miniApps.workshop.releases.rollbackUnavailable')
              }
              disabled={canRollback}
            >
              <span>
                <Button
                  icon={<Undo theme='outline' size='14' />}
                  aria-label={namedAction(t('miniApps.actions.rollback'))}
                  title={
                    canRollback
                      ? undefined
                      : t('miniApps.workshop.releases.rollbackUnavailable')
                  }
                  loading={busyAction === 'rollback'}
                  disabled={controlsDisabled || !canRollback}
                  onClick={onRollback}
                >
                  {t('miniApps.actions.rollback')}
                </Button>
              </span>
            </Tooltip>
            {miniapp.lifecycle === 'enabled' ? (
              <Button
                status='danger'
                icon={<Power theme='outline' size='14' />}
                aria-label={namedAction(t('miniApps.actions.disable'))}
                loading={busyAction === 'disable'}
                disabled={controlsDisabled || !canDisable}
                onClick={() => onSetEnabled(false)}
              >
                {t('miniApps.actions.disable')}
              </Button>
            ) : (
              <Tooltip
                content={
                  miniapp.releases.active
                    ? ''
                    : t('miniApps.errors.enableUnavailable')
                }
                disabled={Boolean(miniapp.releases.active)}
              >
                <span>
                  <Button
                    type={primaryAction === 'enable' ? 'primary' : 'default'}
                    icon={<Power theme='outline' size='14' />}
                    aria-label={namedAction(t('miniApps.actions.enable'))}
                    title={
                      miniapp.releases.active
                        ? undefined
                        : t('miniApps.errors.enableUnavailable')
                    }
                    loading={busyAction === 'enable'}
                    disabled={controlsDisabled || !canEnable}
                    onClick={() => onSetEnabled(true)}
                  >
                    {t('miniApps.actions.enable')}
                  </Button>
                </span>
              </Tooltip>
            )}
            {canTrash && (
              <Button
                icon={<Delete theme='outline' size='14' />}
                aria-label={namedAction(t('miniApps.actions.trash'))}
                loading={busyAction === 'trash'}
                disabled={controlsDisabled}
                onClick={onTrash}
              >
                {t('miniApps.actions.trash')}
              </Button>
            )}
            <Button
              icon={<ShareOne theme='outline' size='14' />}
              aria-label={namedAction(t('miniApps.actions.share'))}
              disabled={controlsDisabled || !canShare}
              onClick={onShare}
            >
              {t('miniApps.actions.share')}
            </Button>
            {miniapp.lifecycle === 'disabled' && (
              <Button
                icon={<Download theme='outline' size='14' />}
                aria-label={namedAction(t('miniApps.actions.exportBackup'))}
                disabled={controlsDisabled}
                onClick={onBackup}
              >
                {t('miniApps.actions.exportBackup')}
              </Button>
            )}
            <Tooltip
              content={
                canOpenSurface
                  ? ''
                  : t('miniApps.errors.surfaceUnavailable')
              }
              disabled={canOpenSurface}
            >
              <span>
                <Button
                  type={primaryAction === 'surface' ? 'primary' : 'default'}
                  icon={<PreviewOpen theme='outline' size='14' />}
                  aria-label={namedAction(t('miniApps.actions.openSurface'))}
                  title={
                    canOpenSurface
                      ? undefined
                      : t('miniApps.errors.surfaceUnavailable')
                  }
                  loading={busyAction === 'open_surface'}
                  disabled={controlsDisabled || !canOpenSurface}
                  onClick={onOpenSurface}
                >
                  {t('miniApps.actions.openSurface')}
                </Button>
              </span>
            </Tooltip>
            {buildRunning && operation.cancelable && (
              <Button
                status='danger'
                icon={<CloseOne theme='outline' size='14' />}
                aria-label={namedAction(t('miniApps.actions.cancelBuild'))}
                loading={canceling}
                disabled={canceling || busyAction !== null}
                onClick={onCancelBuild}
              >
                {t('miniApps.actions.cancelBuild')}
              </Button>
            )}
          </>
        )}
        {canRestore && (
          <Button
            icon={<Undo theme='outline' size='14' />}
            aria-label={namedAction(t('miniApps.actions.restore'))}
            loading={busyAction === 'restore'}
            disabled={controlsDisabled}
            onClick={onRestore}
          >
            {t('miniApps.actions.restore')}
          </Button>
        )}
        {canDelete && (
          <Button
            status='danger'
            icon={<Delete theme='outline' size='14' />}
            aria-label={namedAction(t('miniApps.actions.deletePermanently'))}
            loading={busyAction === 'delete'}
            disabled={controlsDisabled}
            onClick={onDelete}
          >
            {t('miniApps.actions.deletePermanently')}
          </Button>
        )}
        {canRetryDelete && (
          <Button
            status='danger'
            icon={<Refresh theme='outline' size='14' />}
            aria-label={namedAction(t('miniApps.actions.retryDelete'))}
            loading={busyAction === 'retry_delete'}
            disabled={controlsDisabled}
            onClick={onRetryDelete}
          >
            {t('miniApps.actions.retryDelete')}
          </Button>
        )}
        <Button
          icon={<Refresh theme='outline' size='14' />}
          aria-label={namedAction(t('miniApps.actions.refresh'))}
          loading={refreshing}
          disabled={busyAction !== null}
          onClick={onRefresh}
        >
          {t('miniApps.actions.refresh')}
        </Button>
      </div>
      {miniapp.lifecycle === 'trashed' && (
        <div
          className={`${styles.notice} ${styles.noticeWarning}`}
          role='status'
          aria-live='polite'
        >
          {t('miniApps.workshop.deletion.trashedNotice')}
        </div>
      )}
      {permanentDeleteRunning && (
        <div
          className={`${styles.notice} ${styles.noticeInfo}`}
          role='status'
          aria-live='polite'
        >
          {t('miniApps.workshop.deletion.runningNotice')}
        </div>
      )}
      {permanentDeleteFailed && (
        <div
          className={`${styles.notice} ${styles.noticeError}`}
          role='alert'
          aria-live='assertive'
        >
          {t('miniApps.workshop.deletion.failedNotice')}
        </div>
      )}

      <ol
        className={styles.workflow}
        aria-label={t('miniApps.workshop.workflow.ariaLabel')}
      >
        {workflowSteps.map((step, index) => (
          <li
            key={step.key}
            className={`${styles.workflowStep} ${
              styles[`workflowStep_${step.state}`]
            }`}
            aria-current={step.state === 'active' ? 'step' : undefined}
          >
            <span className={styles.workflowIndex} aria-hidden='true'>
              {step.state === 'done' ? (
                <CheckOne theme='outline' size='13' />
              ) : (
                <WorkflowIcon state={step.state} />
              )}
            </span>
            <span>
              {index + 1}. {step.label}
            </span>
          </li>
        ))}
      </ol>

      <section
        className={styles.section}
        aria-labelledby={sourceBuildTitleId}
        aria-describedby={sourceBuildHintId}
      >
        <div className={styles.sectionHeader}>
          <div>
            <h3 id={sourceBuildTitleId} className={styles.sectionTitle}>
              {t('miniApps.workshop.sourceBuild.title')}
            </h3>
            <p id={sourceBuildHintId} className={styles.sectionHint}>
              {t('miniApps.workshop.sourceBuild.hint')}
            </p>
          </div>
          {buildRunning && (
            <StatusBadge
              label={t('miniApps.workshop.operation.state.running')}
              tone='info'
            />
          )}
        </div>
        <div className={styles.factGrid}>
          <Fact
            label={t('miniApps.workshop.identity.projectRevision')}
            value={workshop.project_revision}
          />
          <Fact
            label={t('miniApps.workshop.identity.buildGeneration')}
            value={workshop.build_generation}
          />
          <Fact
            label={t('miniApps.workshop.identity.updatedAt')}
            value={formatMiniAppTimestamp(miniapp.updated_at_ms, locale)}
          />
          <Fact
            label={t('miniApps.workshop.identity.sourceDigest')}
            value={shortMiniAppIdentity(workshop.source_snapshot_digest)}
            title={workshop.source_snapshot_digest}
            mono
          />
          <Fact
            label={t('miniApps.workshop.identity.lockDigest')}
            value={shortMiniAppIdentity(workshop.dependency_lock_digest)}
            title={workshop.dependency_lock_digest}
            mono
          />
        </div>
        {workshop.source_state === 'empty' && (
          <div
            className={`${styles.notice} ${styles.noticeWarning}`}
            role='status'
            aria-live='polite'
          >
            {t('miniApps.workshop.source.emptyNotice')}
          </div>
        )}
        {workshop.source_state === 'runtime_only' && (
          <div
            className={`${styles.notice} ${styles.noticeWarning}`}
            role='status'
            aria-live='polite'
          >
            {t('miniApps.workshop.source.runtimeOnlyNotice')}
          </div>
        )}
      </section>

      <section
        className={styles.section}
        aria-labelledby={releasesTitleId}
        aria-describedby={releasesHintId}
      >
        <div className={styles.sectionHeader}>
          <div>
            <h3 id={releasesTitleId} className={styles.sectionTitle}>
              {t('miniApps.workshop.releases.title')}
            </h3>
            <p id={releasesHintId} className={styles.sectionHint}>
              {t('miniApps.workshop.releases.hint')}
            </p>
          </div>
        </div>
        <div className={styles.releaseGrid}>
          <div className={`${styles.releaseItem} ${styles.releaseItemActive}`}>
            <div className={styles.releaseName}>
              {t('miniApps.workshop.releases.active')}
            </div>
            <div className={styles.releaseValue}>
              {miniapp.releases.active
                ? shortMiniAppIdentity(miniapp.releases.active.release_id)
                : t('miniApps.common.none')}
            </div>
            {miniapp.releases.active && (
              <div
                className={styles.releaseDigest}
                title={miniapp.releases.active.release_digest}
              >
                {shortMiniAppIdentity(
                  miniapp.releases.active.release_digest,
                  12
                )}
              </div>
            )}
          </div>
          <div className={`${styles.releaseItem} ${styles.releaseItemPrevious}`}>
            <div className={styles.releaseName}>
              {t('miniApps.workshop.releases.previous')}
            </div>
            <div className={styles.releaseValue}>
              {miniapp.releases.previous
                ? shortMiniAppIdentity(miniapp.releases.previous.release_id)
                : t('miniApps.common.none')}
            </div>
            {miniapp.releases.previous && (
              <div
                className={styles.releaseDigest}
                title={miniapp.releases.previous.release_digest}
              >
                {shortMiniAppIdentity(
                  miniapp.releases.previous.release_digest,
                  12
                )}
              </div>
            )}
          </div>
          <Fact
            label={t('miniApps.workshop.identity.pointerRevision')}
            value={miniapp.releases.pointer_revision}
          />
          <Fact
            label={t('miniApps.workshop.identity.activeEpoch')}
            value={miniapp.releases.active_release_epoch}
          />
        </div>
        <div
          className={`${styles.notice} ${styles.noticeInfo}`}
          role='status'
          aria-live='polite'
        >
          {t('miniApps.workshop.releases.publishDoesNotEnable')}
        </div>
      </section>

      {miniapp.kind === 'service' && (
        <section
          className={styles.section}
          aria-labelledby={serviceTitleId}
          aria-describedby={serviceHintId}
        >
          <div className={styles.sectionHeader}>
            <div>
              <h3 id={serviceTitleId} className={styles.sectionTitle}>
                {t('miniApps.workshop.service.title')}
              </h3>
              <p id={serviceHintId} className={styles.sectionHint}>
                {t('miniApps.workshop.service.hint')}
              </p>
            </div>
            <MiniAppHealthBadge health={miniapp.service_health} />
          </div>
          <div className={styles.serviceGrid}>
            <Fact
              label={t('miniApps.workshop.service.lifecycle')}
              value={
                serviceLifecycle === 'continuous'
                  ? t('miniApps.workshop.service.lifecycleValue.continuous')
                  : t('miniApps.workshop.service.lifecycleValue.onDemand')
              }
            />
            <Fact
              label={t('miniApps.workshop.service.files')}
              value={
                activeService
                  ? activeService.uses_files
                    ? t('miniApps.common.yes')
                    : t('miniApps.common.no')
                  : t('miniApps.common.unknown')
              }
            />
            <Fact
              label={t('miniApps.workshop.service.privateDatabase')}
              value={
                activeService
                  ? activeService.uses_private_database
                    ? t('miniApps.common.yes')
                    : t('miniApps.common.no')
                  : t('miniApps.common.unknown')
              }
            />
          </div>
          <div className={styles.publishModeControl}>
            <Radio.Group
              type='button'
              size='small'
              aria-label={t('miniApps.workshop.service.lifecycle')}
              value={serviceLifecycle}
              disabled={controlsDisabled || buildRunning || !lifecycleActive}
              onChange={(value: unknown) => {
                if (value === 'on_demand' || value === 'continuous') {
                  onSetServiceLifecycle(value);
                }
              }}
              options={[
                {
                  label: t(
                    'miniApps.workshop.service.lifecycleValue.onDemand'
                  ),
                  value: 'on_demand',
                },
                {
                  label: t(
                    'miniApps.workshop.service.lifecycleValue.continuous'
                  ),
                  value: 'continuous',
                },
              ]}
            />
          </div>
          <div
            className={styles.actionBar}
            role='group'
            aria-label={t('miniApps.workshop.service.title')}
          >
            <Button
              icon={<Power theme='outline' size='14' />}
              aria-label={namedAction(t('miniApps.actions.startService'))}
              loading={busyAction === 'service_start'}
              disabled={
                controlsDisabled ||
                miniapp.service_health.state === 'ready' ||
                miniapp.service_health.state === 'starting' ||
                !miniAppSetServiceRunningRequest(workshop, true)
              }
              onClick={() => onSetServiceRunning(true)}
            >
              {t('miniApps.actions.startService')}
            </Button>
            <Button
              status='danger'
              icon={<Power theme='outline' size='14' />}
              aria-label={namedAction(t('miniApps.actions.stopService'))}
              loading={busyAction === 'service_stop'}
              disabled={
                controlsDisabled ||
                (miniapp.service_health.state !== 'ready' &&
                  miniapp.service_health.state !== 'starting') ||
                !miniAppSetServiceRunningRequest(workshop, false)
              }
              onClick={() => onSetServiceRunning(false)}
            >
              {t('miniApps.actions.stopService')}
            </Button>
            <Button
              icon={<Refresh theme='outline' size='14' />}
              aria-label={namedAction(t('miniApps.actions.retryService'))}
              loading={busyAction === 'service_retry'}
              disabled={
                controlsDisabled ||
                miniapp.service_health.state !== 'failed' ||
                !miniAppRetryServiceRequest(workshop)
              }
              onClick={onRetryService}
            >
              {t('miniApps.actions.retryService')}
            </Button>
          </div>
          {!activeService && (
            <div className={`${styles.notice} ${styles.noticeInfo}`}>
              {t('miniApps.workshop.service.noDescriptor')}
            </div>
          )}
        </section>
      )}

      <section
        className={styles.section}
        aria-labelledby={publishModeTitleId}
        aria-describedby={publishModeHintId}
      >
        <div className={styles.sectionHeader}>
          <div>
            <h3 id={publishModeTitleId} className={styles.sectionTitle}>
              {t('miniApps.workshop.publishMode.title')}
            </h3>
            <p id={publishModeHintId} className={styles.sectionHint}>
              {t('miniApps.workshop.publishMode.hint')}
            </p>
          </div>
          <StatusBadge
            label={
              workshop.publish_mode === 'auto_ui_only'
                ? t('miniApps.workshop.publishMode.auto')
                : t('miniApps.workshop.publishMode.manual')
            }
            tone={
              workshop.publish_mode === 'auto_ui_only' ? 'success' : 'muted'
            }
          />
        </div>
        <span className={styles.publishModeControl}>
          <Radio.Group
            type='button'
            size='small'
            aria-label={t('miniApps.workshop.publishMode.title')}
            value={workshop.publish_mode}
            disabled={
              busyAction !== null || canceling || !autoPublishAvailable
            }
            onChange={(value: unknown) => {
              if (value === 'manual' || value === 'auto_ui_only') {
                onSetPublishMode(value);
              }
            }}
            options={[
              {
                label: t('miniApps.workshop.publishMode.manual'),
                value: 'manual',
              },
              {
                label: t('miniApps.workshop.publishMode.auto'),
                value: 'auto_ui_only',
                disabled: !autoPublishAvailable || building,
              },
            ]}
          />
        </span>
        <p className={styles.publishModeNote}>
          {autoPublishAvailable
            ? t('miniApps.workshop.publishMode.autoScope')
            : t('miniApps.workshop.publishMode.autoDisabled')}
        </p>
      </section>

      <section
        className={styles.section}
        aria-labelledby={readyTitleId}
        aria-describedby={readyHintId}
      >
        <div className={styles.sectionHeader}>
          <div>
            <h3 id={readyTitleId} className={styles.sectionTitle}>
              {t('miniApps.workshop.ready.title')}
            </h3>
            <p id={readyHintId} className={styles.sectionHint}>
              {t('miniApps.workshop.ready.hint')}
            </p>
          </div>
          {ready && (
            <StatusBadge
              label={t('miniApps.workshop.ready.available')}
              tone='success'
            />
          )}
        </div>
        {ready ? (
          <>
            <ReadyReleaseSummary
              release={ready.release}
              buildGeneration={ready.project_build_generation}
              createdAtMs={ready.created_at_ms}
            />
            <div className={`${styles.factGrid} ${styles.readyFacts}`}>
              <Fact
                label={t('miniApps.workshop.ready.test')}
                value={<MiniAppTestBadge status={ready.test.status} />}
              />
              <Fact
                label={t('miniApps.workshop.ready.publishEligibility')}
                value={
                  ready.can_publish
                    ? t('miniApps.workshop.ready.canPublish')
                    : t('miniApps.workshop.ready.blocked')
                }
              />
              <Fact
                label={t('miniApps.workshop.ready.migrations')}
                value={ready.migration_count}
              />
            </div>
            {ready.blocking_reasons.length > 0 && (
              <ul className={styles.blockingList}>
                {ready.blocking_reasons.map((reason) => (
                  <li key={reason} className={styles.blockingItem}>
                    <CloseOne theme='outline' size='13' />
                    <code>{reason}</code>
                  </li>
                ))}
              </ul>
            )}
          </>
        ) : (
          <MiniAppStatePanel
            title={t('miniApps.workshop.ready.emptyTitle')}
            body={t('miniApps.workshop.ready.emptyBody')}
          />
        )}
      </section>

      {surfaceDescriptor && (
        <MiniAppSurfacePanel
          descriptor={surfaceDescriptor}
          displayName={miniapp.display_name}
          reloading={busyAction === 'reload_surface'}
          closing={busyAction === 'close_surface'}
          onReload={onReloadSurface}
          onClose={onCloseSurface}
        />
      )}

      <section
        className={styles.section}
        aria-labelledby={operationTitleId}
        aria-describedby={operationHintId}
        aria-live='polite'
      >
        <div className={styles.sectionHeader}>
          <div>
            <h3 id={operationTitleId} className={styles.sectionTitle}>
              {t('miniApps.workshop.operation.title')}
            </h3>
            <p id={operationHintId} className={styles.sectionHint}>
              {t('miniApps.workshop.operation.hint')}
            </p>
          </div>
        </div>
        {operation ? (
          <div className={styles.operationGrid}>
            <div className={styles.operationItem}>
              <div className={styles.releaseName}>
                {t('miniApps.workshop.operation.kind')}
              </div>
              <div className={styles.releaseValue}>
                {operation.kind === 'miniapp_permanent_delete'
                  ? t(
                      'miniApps.workshop.operation.kindValue.permanentDelete'
                    )
                  : t(
                      `miniApps.workshop.operation.kindValue.${operation.kind}`
                    )
                }
              </div>
            </div>
            <div className={styles.operationItem}>
              <div className={styles.releaseName}>
                {t('miniApps.workshop.operation.status')}
              </div>
              <div className={styles.releaseValue}>
                <OperationBadge state={operation.state} />
              </div>
            </div>
            <div className={styles.operationItem}>
              <div className={styles.releaseName}>
                {t('miniApps.workshop.operation.progress')}
              </div>
              <div className={styles.releaseValue}>
                {operation.progress_percent == null
                  ? t('miniApps.common.unknown')
                  : `${operation.progress_percent}%`}
              </div>
            </div>
            <div className={styles.operationItem}>
              <div className={styles.releaseName}>
                {t('miniApps.workshop.operation.started')}
              </div>
              <div className={styles.releaseValue}>
                {formatMiniAppTimestamp(operation.started_at_ms, locale)}
              </div>
            </div>
          </div>
        ) : (
          <MiniAppStatePanel
            title={t('miniApps.workshop.operation.emptyTitle')}
            body={t('miniApps.workshop.operation.emptyBody')}
          />
        )}
      </section>
    </main>
  );
};

const MiniAppRunnerPage: React.FC = () => {
  const { t, i18n } = useTranslation();
  const navigate = useNavigate();
  const { id: rawId } = useParams<{ id: string }>();
  const miniappId = useMemo(() => {
    if (!rawId) return null;
    try {
      return parseMiniAppId(rawId);
    } catch {
      return null;
    }
  }, [rawId]);
  const [message, messageContext] = useArcoMessage({ maxCount: 3 });
  const [workshop, setWorkshop] = useState<MiniAppWorkshop | null>(null);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [building, setBuilding] = useState(false);
  const [canceling, setCanceling] = useState(false);
  const [serviceLifecycle, setServiceLifecycle] =
    useState<MiniAppServiceLifecycle>('on_demand');
  const [busyAction, setBusyAction] = useState<MiniAppBusyAction>(null);
  const [surfaceDescriptor, setSurfaceDescriptor] =
    useState<MiniAppSurfaceLaunchDescriptor | null>(null);
  const [shareVisible, setShareVisible] = useState(false);
  const [backupVisible, setBackupVisible] = useState(false);
  const [sourceEditVisible, setSourceEditVisible] = useState(false);
  const canceledBuildRef = useRef<string | null>(null);
  const deletionInProgressRef = useRef(false);
  const [notFound, setNotFound] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);

  const finishDeletion = useCallback(
    (successMessage: string) => {
      deletionInProgressRef.current = false;
      setWorkshop(null);
      setSurfaceDescriptor(null);
      setShareVisible(false);
      setBackupVisible(false);
      setSourceEditVisible(false);
      setFailure(null);
      message.success(successMessage);
      navigate('/mini-apps', { replace: true });
    },
    [message, navigate]
  );

  useEffect(() => {
    deletionInProgressRef.current =
      workshop?.miniapp.lifecycle === 'deleting';
  }, [workshop?.miniapp.lifecycle]);

  useEffect(() => {
    if (workshop?.miniapp.kind !== 'service') {
      setServiceLifecycle('on_demand');
      return;
    }
    const persistedLifecycle =
      workshop.service_lifecycle ?? workshop.ready?.service?.lifecycle;
    if (persistedLifecycle) setServiceLifecycle(persistedLifecycle);
  }, [
    workshop?.miniapp.kind,
    workshop?.service_lifecycle,
    workshop?.ready?.release.release_id,
    workshop?.ready?.service?.lifecycle,
  ]);

  const load = useCallback(
    async (
      mode: 'initial' | 'refresh' | 'background' = 'initial'
    ): Promise<MiniAppWorkshop | null> => {
      if (!miniappId) {
        setWorkshop(null);
        setSurfaceDescriptor(null);
        setNotFound(true);
        setFailure(null);
        setLoading(false);
        return null;
      }
      if (mode === 'initial') setLoading(true);
      else if (mode === 'refresh') setRefreshing(true);
      try {
        const next = await ipcBridge.miniapps.getWorkshop.invoke({
          miniapp_id: miniappId,
        });
        setWorkshop(next);
        setNotFound(false);
        if (mode !== 'background') setFailure(null);
        return next;
      } catch (error) {
        if (isBackendHttpError(error) && error.status === 404) {
          if (deletionInProgressRef.current) {
            finishDeletion(t('miniApps.messages.deletedPermanently'));
            return null;
          }
          setWorkshop(null);
          setSurfaceDescriptor(null);
          setNotFound(true);
          setFailure(null);
        } else {
          console.error('[miniapps] failed to load M1 Workshop', error);
          setFailure(formatError(error));
        }
        return null;
      } finally {
        setLoading(false);
        if (mode === 'refresh') setRefreshing(false);
      }
    },
    [finishDeletion, miniappId, t]
  );

  const requestSurfaceDescriptor = useCallback(
    async (
      target: MiniAppWorkshop
    ): Promise<MiniAppSurfaceLaunchDescriptor> => {
      if (
        !miniappId ||
        target.miniapp.miniapp_id !== miniappId ||
        !miniAppCanOpenSurface(target)
      ) {
        throw new Error(t('miniApps.errors.surfaceUnavailable'));
      }
      const descriptor = await ipcBridge.miniapps.openSurface.invoke({
        miniapp_id: miniappId,
      });
      if (
        !miniAppSurfaceMatchesWorkshop(descriptor, target) ||
        !miniAppSurfaceAssetPath(descriptor)
      ) {
        throw new Error(t('miniApps.errors.surfaceDescriptorMismatch'));
      }
      return descriptor;
    },
    [miniappId, t]
  );

  const syncSurface = useCallback(
    async (target: MiniAppWorkshop, shouldOpen: boolean) => {
      if (!shouldOpen || !miniAppCanOpenSurface(target)) {
        setSurfaceDescriptor(null);
        return;
      }
      if (
        surfaceDescriptor &&
        miniAppSurfaceMatchesWorkshop(surfaceDescriptor, target)
      ) {
        return;
      }
      setSurfaceDescriptor(null);
      try {
        setSurfaceDescriptor(await requestSurfaceDescriptor(target));
      } catch (error) {
        console.error('[miniapps] failed to refresh Surface descriptor', error);
        setFailure(
          `${t('miniApps.errors.loadSurfaceTitle')}: ${formatError(error)}`
        );
      }
    },
    [requestSurfaceDescriptor, surfaceDescriptor, t]
  );

  useEffect(() => {
    deletionInProgressRef.current = false;
    setSurfaceDescriptor(null);
    setShareVisible(false);
    setBackupVisible(false);
    setSourceEditVisible(false);
  }, [miniappId]);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    if (
      !workshop ||
      (!building &&
        (workshop.active_operation?.kind !== 'build' ||
          workshop.active_operation.state !== 'running'))
    ) {
      return;
    }
    const timer = window.setTimeout(() => void load('background'), 350);
    return () => window.clearTimeout(timer);
  }, [
    building,
    load,
    workshop?.active_operation?.kind,
    workshop?.active_operation?.operation_id,
    workshop?.active_operation?.state,
  ]);

  const permanentDeleteRunning = isPermanentDeleteRunning(workshop);

  useEffect(() => {
    if (!miniappId || !permanentDeleteRunning) return;
    let canceled = false;
    let timer: number | null = null;

    const poll = async () => {
      try {
        const next = await ipcBridge.miniapps.getWorkshop.invoke({
          miniapp_id: miniappId,
        });
        if (canceled) return;
        deletionInProgressRef.current =
          next.miniapp.lifecycle === 'deleting';
        setWorkshop(next);
        setNotFound(false);
        setFailure(null);
        if (isPermanentDeleteRunning(next)) {
          timer = window.setTimeout(poll, 750);
        }
      } catch (error) {
        if (canceled) return;
        if (isBackendHttpError(error) && error.status === 404) {
          finishDeletion(t('miniApps.messages.deletedPermanently'));
          return;
        }
        console.error(
          '[miniapps] failed to poll permanent deletion',
          error
        );
        setFailure(formatError(error));
        timer = window.setTimeout(poll, 1500);
      }
    };

    timer = window.setTimeout(poll, 750);
    return () => {
      canceled = true;
      if (timer !== null) window.clearTimeout(timer);
    };
  }, [finishDeletion, miniappId, permanentDeleteRunning, t]);

  useEffect(() => {
    if (
      surfaceDescriptor &&
      workshop &&
      (!miniAppSurfaceMatchesWorkshop(surfaceDescriptor, workshop) ||
        !miniAppSurfaceAssetPath(surfaceDescriptor))
    ) {
      setSurfaceDescriptor(null);
    }
  }, [surfaceDescriptor, workshop]);

  const goBack = useCallback(() => navigate('/mini-apps'), [navigate]);

  const runWorkshopMutation = useCallback(
    async (
      action: Exclude<
        MiniAppBusyAction,
        'open_surface' | 'reload_surface' | 'close_surface' | null
      >,
      invoke: () => Promise<MiniAppWorkshop>,
      successMessage: string,
      allowWhileBuilding = false
    ) => {
      if (busyAction || (!allowWhileBuilding && building) || canceling) return;
      const surfaceWasOpen = surfaceDescriptor !== null;
      setBusyAction(action);
      setFailure(null);
      try {
        const next = await invoke();
        setWorkshop(next);
        setNotFound(false);
        message.success(successMessage);
        await syncSurface(next, surfaceWasOpen);
      } catch (error) {
        console.error(`[miniapps] ${action} failed`, error);
        const detail = formatError(error);
        const latest = await load('background');
        if (latest) await syncSurface(latest, surfaceWasOpen);
        setFailure(detail);
      } finally {
        setBusyAction(null);
      }
    },
    [
      building,
      busyAction,
      canceling,
      load,
      message,
      surfaceDescriptor,
      syncSurface,
    ]
  );

  const runPermanentDelete = useCallback(
    async (
      action: 'delete' | 'retry_delete',
      invoke: () => Promise<unknown>,
      successMessage: string
    ) => {
      if (!miniappId || busyAction || building || canceling) return;
      deletionInProgressRef.current = true;
      setBusyAction(action);
      setFailure(null);
      setSurfaceDescriptor(null);
      try {
        await invoke();
        finishDeletion(successMessage);
      } catch (error) {
        console.error(`[miniapps] ${action} failed`, error);
        const detail = formatError(error);
        try {
          const latest = await ipcBridge.miniapps.getWorkshop.invoke({
            miniapp_id: miniappId,
          });
          deletionInProgressRef.current =
            latest.miniapp.lifecycle === 'deleting';
          setWorkshop(latest);
          setNotFound(false);
        } catch (refreshError) {
          if (
            isBackendHttpError(refreshError) &&
            refreshError.status === 404
          ) {
            finishDeletion(successMessage);
            return;
          }
          console.error(
            '[miniapps] failed to reload Workshop after delete failure',
            refreshError
          );
        }
        setFailure(detail);
      } finally {
        setBusyAction(null);
      }
    },
    [
      building,
      busyAction,
      canceling,
      finishDeletion,
      miniappId,
    ]
  );

  const handleBuild = useCallback(async () => {
    if (!workshop || !miniappId || building || canceling || busyAction) {
      return;
    }
    const request = miniAppBuildRequest(workshop, serviceLifecycle);
    if (!request) {
      setFailure(t('miniApps.errors.buildUnavailable'));
      return;
    }
    const surfaceWasOpen = surfaceDescriptor !== null;
    canceledBuildRef.current = null;
    setBuilding(true);
    setFailure(null);
    try {
      const next = await ipcBridge.miniapps.build.invoke(request);
      setWorkshop(next);
      message.success(t('miniApps.messages.buildSucceeded'));
      await syncSurface(next, surfaceWasOpen);
    } catch (error) {
      if (canceledBuildRef.current) {
        canceledBuildRef.current = null;
      } else {
        console.error('[miniapps] UI-only Build failed', error);
        const detail = formatError(error);
        const latest = await load('background');
        if (latest) await syncSurface(latest, surfaceWasOpen);
        setFailure(detail);
      }
    } finally {
      setBuilding(false);
    }
  }, [
    building,
    busyAction,
    canceling,
    load,
    message,
    miniappId,
    serviceLifecycle,
    surfaceDescriptor,
    syncSurface,
    t,
    workshop,
  ]);

  const handleCancelBuild = useCallback(async () => {
    if (!workshop || !miniappId || canceling || busyAction) return;
    const operation = workshop.active_operation;
    if (
      !operation ||
      operation.kind !== 'build' ||
      operation.state !== 'running' ||
      !operation.cancelable
    ) {
      return;
    }
    canceledBuildRef.current = operation.operation_id;
    setCanceling(true);
    setFailure(null);
    try {
      await ipcBridge.miniapps.cancelBuild.invoke({
        miniapp_id: miniappId,
        operation_id: operation.operation_id,
        expected_operation_revision: operation.operation_revision,
      });
      await load('background');
      message.success(t('miniApps.messages.buildCanceled'));
    } catch (error) {
      canceledBuildRef.current = null;
      console.error('[miniapps] canceling UI-only Build failed', error);
      setFailure(formatError(error));
      await load('background');
    } finally {
      setCanceling(false);
    }
  }, [busyAction, canceling, load, message, miniappId, t, workshop]);

  const handlePublish = useCallback(() => {
    if (!workshop || busyAction || building || canceling) return;
    const request = miniAppPublishRequest(workshop);
    if (!request) {
      setFailure(t('miniApps.errors.publishUnavailable'));
      return;
    }
    Modal.confirm({
      title: t('miniApps.confirm.publishTitle'),
      content:
        workshop.miniapp.kind === 'service'
          ? t('miniApps.confirm.publishServiceBody')
          : workshop.miniapp.releases.active
            ? t('miniApps.confirm.publishUpdateBody')
            : t('miniApps.confirm.publishFirstBody'),
      okText: t('miniApps.actions.publish'),
      cancelText: t('miniApps.actions.cancel'),
      onOk: () =>
        runWorkshopMutation(
          'publish',
          () => ipcBridge.miniapps.publish.invoke(request),
          t('miniApps.messages.published')
        ),
    });
  }, [building, busyAction, canceling, runWorkshopMutation, t, workshop]);

  const handleTest = useCallback(() => {
    if (!workshop || busyAction || building || canceling) return;
    const request = miniAppTestRequest(workshop);
    if (!request) {
      setFailure(t('miniApps.errors.testUnavailable'));
      return;
    }
    Modal.confirm({
      title: t('miniApps.confirm.testTitle'),
      content: t('miniApps.confirm.testBody'),
      okText: t('miniApps.actions.testService'),
      cancelText: t('miniApps.actions.cancel'),
      okButtonProps: { status: 'warning' },
      onOk: () =>
        runWorkshopMutation(
          'test',
          () => ipcBridge.miniapps.test.invoke(request),
          t('miniApps.messages.testCompleted')
        ),
    });
  }, [building, busyAction, canceling, runWorkshopMutation, t, workshop]);

  const handleSetServiceRunning = useCallback(
    (running: boolean) => {
      if (!workshop || busyAction || building || canceling) return;
      const request = miniAppSetServiceRunningRequest(workshop, running);
      if (!request) {
        setFailure(t('miniApps.errors.serviceUnavailable'));
        return;
      }
      void runWorkshopMutation(
        running ? 'service_start' : 'service_stop',
        () => ipcBridge.miniapps.setServiceRunning.invoke(request),
        t(
          running
            ? 'miniApps.messages.serviceStarted'
            : 'miniApps.messages.serviceStopped'
        )
      );
    },
    [building, busyAction, canceling, runWorkshopMutation, t, workshop]
  );

  const handleRetryService = useCallback(() => {
    if (!workshop || busyAction || building || canceling) return;
    const request = miniAppRetryServiceRequest(workshop);
    if (!request) {
      setFailure(t('miniApps.errors.serviceUnavailable'));
      return;
    }
    void runWorkshopMutation(
      'service_retry',
      () => ipcBridge.miniapps.retryService.invoke(request),
      t('miniApps.messages.serviceRetried')
    );
  }, [building, busyAction, canceling, runWorkshopMutation, t, workshop]);

  const handleRollback = useCallback(() => {
    if (!workshop || busyAction || building || canceling) return;
    const request = miniAppRollbackRequest(workshop);
    if (!request) {
      setFailure(t('miniApps.errors.rollbackUnavailable'));
      return;
    }
    Modal.confirm({
      title: t('miniApps.confirm.rollbackTitle'),
      content: t('miniApps.confirm.rollbackBody', {
        current: shortMiniAppIdentity(
          request.expected_current_release_digest,
          8
        ),
        target: shortMiniAppIdentity(
          request.expected_previous_release_digest,
          8
        ),
      }),
      okText: t('miniApps.actions.rollback'),
      cancelText: t('miniApps.actions.cancel'),
      okButtonProps: { status: 'warning' },
      onOk: () =>
        runWorkshopMutation(
          'rollback',
          () => ipcBridge.miniapps.rollback.invoke(request),
          t('miniApps.messages.rolledBack')
        ),
    });
  }, [building, busyAction, canceling, runWorkshopMutation, t, workshop]);

  const handleSetEnabled = useCallback(
    (enabled: boolean) => {
      if (!workshop || busyAction || building || canceling) return;
      const request = miniAppSetEnabledRequest(workshop, enabled);
      if (!request) {
        setFailure(
          t(
            enabled
              ? 'miniApps.errors.enableUnavailable'
              : 'miniApps.errors.disableUnavailable'
          )
        );
        return;
      }
      const run = () =>
        runWorkshopMutation(
          enabled ? 'enable' : 'disable',
          () => ipcBridge.miniapps.setEnabled.invoke(request),
          t(
            enabled
              ? 'miniApps.messages.enabled'
              : 'miniApps.messages.disabled'
          )
        );
      if (enabled) {
        void run();
        return;
      }
      Modal.confirm({
        title: t('miniApps.confirm.disableTitle'),
        content: t('miniApps.confirm.disableBody'),
        okText: t('miniApps.actions.disable'),
        cancelText: t('miniApps.actions.cancel'),
        okButtonProps: { status: 'danger' },
        onOk: run,
      });
    },
    [building, busyAction, canceling, runWorkshopMutation, t, workshop]
  );

  const handleTrash = useCallback(() => {
    if (!workshop || busyAction || building || canceling) return;
    const request = miniAppTrashRequest(workshop);
    if (!request) {
      setFailure(t('miniApps.errors.trashUnavailable'));
      return;
    }
    Modal.confirm({
      title: t('miniApps.confirm.trashTitle', {
        name: workshop.miniapp.display_name,
      }),
      content: t('miniApps.confirm.trashBody'),
      okText: t('miniApps.actions.trash'),
      cancelText: t('miniApps.actions.cancel'),
      okButtonProps: { status: 'warning' },
      onOk: () =>
        runWorkshopMutation(
          'trash',
          () => ipcBridge.miniapps.trash.invoke(request),
          t('miniApps.messages.trashed')
        ),
    });
  }, [building, busyAction, canceling, runWorkshopMutation, t, workshop]);

  const handleRestore = useCallback(() => {
    if (!workshop || busyAction || building || canceling) return;
    const request = miniAppRestoreRequest(workshop);
    if (!request) {
      setFailure(t('miniApps.errors.restoreUnavailable'));
      return;
    }
    Modal.confirm({
      title: t('miniApps.confirm.restoreTitle', {
        name: workshop.miniapp.display_name,
      }),
      content: t('miniApps.confirm.restoreBody'),
      okText: t('miniApps.actions.restore'),
      cancelText: t('miniApps.actions.cancel'),
      onOk: () =>
        runWorkshopMutation(
          'restore',
          () => ipcBridge.miniapps.restore.invoke(request),
          t('miniApps.messages.restored')
        ),
    });
  }, [building, busyAction, canceling, runWorkshopMutation, t, workshop]);

  const handleDelete = useCallback(() => {
    if (!workshop || busyAction || building || canceling) return;
    const request = miniAppDeleteRequest(workshop);
    if (!request) {
      setFailure(t('miniApps.errors.deleteUnavailable'));
      return;
    }
    Modal.confirm({
      title: t('miniApps.confirm.deleteTitle', {
        name: workshop.miniapp.display_name,
      }),
      content: t('miniApps.confirm.deleteBody'),
      okText: t('miniApps.actions.deletePermanently'),
      cancelText: t('miniApps.actions.cancel'),
      okButtonProps: { status: 'danger' },
      onOk: () =>
        runPermanentDelete(
          'delete',
          () => ipcBridge.miniapps.delete.invoke(request),
          t('miniApps.messages.deletedPermanently')
        ),
    });
  }, [building, busyAction, canceling, runPermanentDelete, t, workshop]);

  const handleRetryDelete = useCallback(() => {
    if (!workshop || busyAction || building || canceling) return;
    const request = miniAppRetryDeleteRequest(workshop);
    if (!request) {
      setFailure(t('miniApps.errors.retryDeleteUnavailable'));
      return;
    }
    Modal.confirm({
      title: t('miniApps.confirm.retryDeleteTitle', {
        name: workshop.miniapp.display_name,
      }),
      content: t('miniApps.confirm.retryDeleteBody'),
      okText: t('miniApps.actions.retryDelete'),
      cancelText: t('miniApps.actions.cancel'),
      okButtonProps: { status: 'danger' },
      onOk: () =>
        runPermanentDelete(
          'retry_delete',
          () => ipcBridge.miniapps.retryDelete.invoke(request),
          t('miniApps.messages.deleteRetryCompleted')
        ),
    });
  }, [building, busyAction, canceling, runPermanentDelete, t, workshop]);

  const handleSetPublishMode = useCallback(
    (mode: MiniAppPublishMode) => {
      if (
        !workshop ||
        busyAction ||
        canceling ||
        (building && mode !== 'manual')
      ) {
        return;
      }
      const request = miniAppSetPublishModeRequest(workshop, mode);
      if (!request) {
        if (
          mode === 'auto_ui_only' &&
          !workshop.miniapp.releases.active
        ) {
          setFailure(t('miniApps.errors.autoPublishRequiresActive'));
        }
        return;
      }
      void runWorkshopMutation(
        'publish_mode',
        () => ipcBridge.miniapps.setPublishMode.invoke(request),
        t(
          mode === 'auto_ui_only'
            ? 'miniApps.messages.autoPublishEnabled'
            : 'miniApps.messages.manualPublishEnabled'
        ),
        mode === 'manual'
      );
    },
    [building, busyAction, canceling, runWorkshopMutation, t, workshop]
  );

  const handleOpenSurface = useCallback(async () => {
    if (
      !workshop ||
      busyAction ||
      building ||
      canceling ||
      !miniAppCanOpenSurface(workshop)
    ) {
      if (workshop && !miniAppCanOpenSurface(workshop)) {
        setFailure(t('miniApps.errors.surfaceUnavailable'));
      }
      return;
    }
    setBusyAction('open_surface');
    setFailure(null);
    setSurfaceDescriptor(null);
    try {
      setSurfaceDescriptor(await requestSurfaceDescriptor(workshop));
    } catch (error) {
      console.error('[miniapps] failed to open Surface', error);
      setFailure(formatError(error));
    } finally {
      setBusyAction(null);
    }
  }, [
    building,
    busyAction,
    canceling,
    requestSurfaceDescriptor,
    t,
    workshop,
  ]);

  const handleReloadSurface = useCallback(async () => {
    if (
      !workshop ||
      !surfaceDescriptor ||
      busyAction ||
      building ||
      canceling
    ) {
      return;
    }
    setBusyAction('reload_surface');
    setFailure(null);
    try {
      setSurfaceDescriptor(await requestSurfaceDescriptor(workshop));
    } catch (error) {
      console.error('[miniapps] failed to reload Surface', error);
      setSurfaceDescriptor(null);
      setFailure(formatError(error));
    } finally {
      setBusyAction(null);
    }
  }, [
    building,
    busyAction,
    canceling,
    requestSurfaceDescriptor,
    surfaceDescriptor,
    workshop,
  ]);

  const handleCloseSurface = useCallback(async () => {
    const descriptor = surfaceDescriptor;
    if (
      !descriptor ||
      busyAction ||
      building ||
      canceling
    ) {
      return;
    }
    setBusyAction('close_surface');
    setFailure(null);
    try {
      await ipcBridge.miniapps.closeSurface.invoke({
        miniapp_id: descriptor.miniapp_id,
        surface_session_id: descriptor.surface_session_id,
        surface_capability: descriptor.surface_capability,
      });
      // Keep the descriptor mounted until the Host confirms the close. This
      // preserves a retryable UI after a transient transport failure and lets
      // the iframe cleanup run from the normal descriptor-unmount path.
      setSurfaceDescriptor(null);
    } catch (error) {
      console.error('[miniapps] failed to close Surface session', error);
      setFailure(formatError(error));
    } finally {
      setBusyAction(null);
    }
  }, [building, busyAction, canceling, surfaceDescriptor]);

  const handleRefresh = useCallback(async () => {
    const surfaceWasOpen = surfaceDescriptor !== null;
    const next = await load('refresh');
    if (next) await syncSurface(next, surfaceWasOpen);
  }, [load, surfaceDescriptor, syncSurface]);

  const handleShareExported = useCallback(
    (_operation: unknown, destinationPath: string) => {
      setShareVisible(false);
      message.success(
        t('miniApps.messages.shareExported', { path: destinationPath })
      );
      void load('background');
    },
    [load, message, t]
  );

  return (
    <>
      {messageContext}
      <HubPageShell
        title={t('miniApps.workshop.pageTitle')}
        subtitle={t('miniApps.workshop.pageSubtitle')}
        className={styles.page}
        maxWidthClass='md:max-w-1200px'
      >
        {loading && !workshop ? (
          <MiniAppStatePanel
            loading
            title=''
            body={t('miniApps.states.loadingWorkshop')}
          />
        ) : notFound ? (
          <MiniAppStatePanel
            title={t('miniApps.errors.notFoundTitle')}
            body={t('miniApps.errors.notFoundBody')}
            onRetry={goBack}
          />
        ) : failure && !workshop ? (
          <MiniAppStatePanel
            title={t('miniApps.errors.loadWorkshopTitle')}
            body={failure}
            onRetry={() => void load('initial')}
          />
        ) : workshop ? (
          <>
            {failure && (
              <div
                className={`${styles.notice} ${styles.noticeError}`}
                role='alert'
                aria-live='assertive'
              >
                <span className={styles.noticeMessage}>{failure}</span>
                <Button
                  size='small'
                  icon={<Refresh theme='outline' size='14' />}
                  aria-label={`${t('miniApps.actions.retry')}: ${workshop.miniapp.display_name}`}
                  loading={refreshing}
                  disabled={refreshing || busyAction !== null}
                  className={styles.noticeAction}
                  onClick={() => void handleRefresh()}
                >
                  {t('miniApps.actions.retry')}
                </Button>
              </div>
            )}
            <MiniAppWorkshopDetail
              workshop={workshop}
              locale={i18n.language}
              onBack={goBack}
              onRefresh={() => void handleRefresh()}
              onEditSource={() => setSourceEditVisible(true)}
              onBuild={() => void handleBuild()}
              onTest={handleTest}
              onCancelBuild={() => void handleCancelBuild()}
              onPublish={handlePublish}
              onRollback={handleRollback}
              onSetEnabled={handleSetEnabled}
              onSetPublishMode={handleSetPublishMode}
              onSetServiceLifecycle={setServiceLifecycle}
              onSetServiceRunning={handleSetServiceRunning}
              onRetryService={handleRetryService}
              onTrash={handleTrash}
              onRestore={handleRestore}
              onDelete={handleDelete}
              onRetryDelete={handleRetryDelete}
              onShare={() => setShareVisible(true)}
              onBackup={() => setBackupVisible(true)}
              onOpenSurface={() => void handleOpenSurface()}
              onReloadSurface={() => void handleReloadSurface()}
              onCloseSurface={() => void handleCloseSurface()}
              surfaceDescriptor={surfaceDescriptor}
              busyAction={busyAction}
              refreshing={refreshing}
              building={building}
              canceling={canceling}
              serviceLifecycle={serviceLifecycle}
            />
          </>
        ) : null}
      </HubPageShell>
      <MiniAppSourceEditDialog
        visible={sourceEditVisible}
        workshop={workshop}
        onCancel={() => setSourceEditVisible(false)}
        onSaved={(updated) => {
          setWorkshop(updated);
          setSourceEditVisible(false);
          setFailure(null);
          message.success(t('miniApps.messages.sourceSaved'));
        }}
      />
      <MiniAppTransferDialog
        mode='export'
        visible={shareVisible}
        libraryRevision={0}
        workshop={workshop}
        onCancel={() => setShareVisible(false)}
        onImported={() => undefined}
        onExported={handleShareExported}
      />
      <MiniAppTransferDialog
        mode='export_backup'
        visible={backupVisible}
        libraryRevision={0}
        workshop={workshop}
        onCancel={() => setBackupVisible(false)}
        onImported={() => undefined}
        onExported={(_operation, destinationPath) => {
          setBackupVisible(false);
          message.success(
            t('miniApps.messages.backupExported', { path: destinationPath })
          );
        }}
      />
    </>
  );
};

export default MiniAppRunnerPage;
