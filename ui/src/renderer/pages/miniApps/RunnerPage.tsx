/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import type {
  MiniAppOperationState,
  MiniAppReleaseRef,
  MiniAppWorkshop,
} from '@/common/types/miniAppPlatform';
import { parseMiniAppId } from '@/common/types/ids';
import HubPageShell from '@/renderer/components/layout/HubPageShell';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import { Button } from '@arco-design/web-react';
import {
  ArrowLeft,
  CheckOne,
  CloseOne,
  Code,
  Refresh,
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
  miniAppWorkflowState,
  shortMiniAppIdentity,
} from './model';
import {
  MiniAppKindBadge,
  MiniAppLifecycleBadge,
  MiniAppSourceBadge,
  MiniAppStatePanel,
  StatusBadge,
  WorkflowIcon,
} from './MiniAppM1State';
import styles from './MiniAppWorkbench.module.css';

function formatError(error: unknown): string {
  if (isBackendHttpError(error)) {
    const detail = error.backendMessage || error.message;
    return error.code ? `${error.code}: ${detail}` : detail;
  }
  return error instanceof Error ? error.message : String(error);
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
      {value || '—'}
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

const MiniAppWorkshopDetail: React.FC<{
  workshop: MiniAppWorkshop;
  locale: string;
  onBack: () => void;
  onRefresh: () => void;
  onBuild: () => void;
  onCancelBuild: () => void;
  refreshing: boolean;
  building: boolean;
  canceling: boolean;
}> = ({
  workshop,
  locale,
  onBack,
  onRefresh,
  onBuild,
  onCancelBuild,
  refreshing,
  building,
  canceling,
}) => {
  const { t } = useTranslation();
  const { miniapp, ready, active_operation: operation } = workshop;
  const workflow = miniAppWorkflowState(workshop);
  const buildRunning =
    operation?.kind === 'build' && operation.state === 'running';
  const canBuild = miniAppBuildRequest(workshop) !== null;
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
  ] as const;

  return (
    <main className={styles.detail}>
      <header className={styles.detailHeader}>
        <div className={styles.detailHeaderCopy}>
          <span className={styles.eyebrow}>
            {t('miniApps.workshop.detailEyebrow')}
          </span>
          <div className={styles.detailTitleRow}>
            <h2 className={styles.detailTitle}>{miniapp.display_name}</h2>
            <MiniAppKindBadge kind={miniapp.kind} />
            <MiniAppLifecycleBadge lifecycle={miniapp.lifecycle} />
            <MiniAppSourceBadge source={workshop.source_state} />
          </div>
          <p className={styles.detailDescription}>
            {miniapp.description || t('miniApps.library.noDescription')}
          </p>
        </div>
      </header>

      <div className={styles.actionBar}>
        <Button
          icon={<ArrowLeft theme='outline' size='14' />}
          onClick={onBack}
        >
          {t('miniApps.actions.backToLibrary')}
        </Button>
        <Button
          type='primary'
          icon={<Code theme='outline' size='14' />}
          loading={building}
          disabled={!canBuild || building || canceling}
          onClick={onBuild}
        >
          {t('miniApps.actions.build')}
        </Button>
        {buildRunning && operation.cancelable && (
          <Button
            status='danger'
            icon={<CloseOne theme='outline' size='14' />}
            loading={canceling}
            disabled={canceling}
            onClick={onCancelBuild}
          >
            {t('miniApps.actions.cancelBuild')}
          </Button>
        )}
        <Button
          icon={<Refresh theme='outline' size='14' />}
          loading={refreshing}
          onClick={onRefresh}
        >
          {t('miniApps.actions.refresh')}
        </Button>
      </div>

      <ol
        className={`${styles.workflow} ${styles.workflowCompact}`}
        aria-label={t('miniApps.workshop.workflow.ariaLabel')}
      >
        {workflowSteps.map((step, index) => (
          <li
            key={step.key}
            className={`${styles.workflowStep} ${
              styles[`workflowStep_${step.state}`]
            }`}
          >
            <span className={styles.workflowIndex}>
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

      <section className={styles.section}>
        <div className={styles.sectionHeader}>
          <div>
            <h3 className={styles.sectionTitle}>
              {t('miniApps.workshop.sourceBuild.title')}
            </h3>
            <p className={styles.sectionHint}>
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
          <div className={`${styles.notice} ${styles.noticeWarning}`}>
            {t('miniApps.workshop.source.emptyNotice')}
          </div>
        )}
        {workshop.source_state === 'runtime_only' && (
          <div className={`${styles.notice} ${styles.noticeWarning}`}>
            {t('miniApps.workshop.source.runtimeOnlyNotice')}
          </div>
        )}
        {miniapp.kind !== 'ui_only' && (
          <div className={`${styles.notice} ${styles.noticeWarning}`}>
            {t('miniApps.workshop.source.serviceDeferred')}
          </div>
        )}
      </section>

      <section className={styles.section}>
        <div className={styles.sectionHeader}>
          <div>
            <h3 className={styles.sectionTitle}>
              {t('miniApps.workshop.ready.title')}
            </h3>
            <p className={styles.sectionHint}>
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
          <ReadyReleaseSummary
            release={ready.release}
            buildGeneration={ready.project_build_generation}
            createdAtMs={ready.created_at_ms}
          />
        ) : (
          <MiniAppStatePanel
            title={t('miniApps.workshop.ready.emptyTitle')}
            body={t('miniApps.workshop.ready.emptyBody')}
          />
        )}
      </section>

      <section className={styles.section}>
        <div className={styles.sectionHeader}>
          <div>
            <h3 className={styles.sectionTitle}>
              {t('miniApps.workshop.operation.title')}
            </h3>
            <p className={styles.sectionHint}>
              {t('miniApps.workshop.operation.hint')}
            </p>
          </div>
        </div>
        {operation ? (
          <div className={styles.operationGrid}>
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
  const canceledBuildRef = useRef<string | null>(null);
  const [notFound, setNotFound] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);

  const load = useCallback(
    async (mode: 'initial' | 'refresh' | 'background' = 'initial') => {
      if (!miniappId) {
        setWorkshop(null);
        setNotFound(true);
        setFailure(null);
        setLoading(false);
        return;
      }
      if (mode === 'initial') setLoading(true);
      else if (mode === 'refresh') setRefreshing(true);
      try {
        const next = await ipcBridge.miniapps.getWorkshop.invoke({
          miniapp_id: miniappId,
        });
        setWorkshop(next);
        setNotFound(false);
        setFailure(null);
      } catch (error) {
        if (isBackendHttpError(error) && error.status === 404) {
          setWorkshop(null);
          setNotFound(true);
          setFailure(null);
        } else {
          console.error('[miniapps] failed to load M1 Workshop', error);
          setFailure(formatError(error));
        }
      } finally {
        setLoading(false);
        if (mode === 'refresh') setRefreshing(false);
      }
    },
    [miniappId]
  );

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

  const goBack = useCallback(() => navigate('/mini-apps'), [navigate]);

  const handleBuild = useCallback(async () => {
    if (!workshop || !miniappId || building || canceling) return;
    const request = miniAppBuildRequest(workshop);
    if (!request) {
      setFailure(t('miniApps.errors.buildUnavailable'));
      return;
    }
    canceledBuildRef.current = null;
    setBuilding(true);
    setFailure(null);
    try {
      const next = await ipcBridge.miniapps.build.invoke(request);
      setWorkshop(next);
      message.success(t('miniApps.messages.buildSucceeded'));
    } catch (error) {
      if (canceledBuildRef.current) {
        canceledBuildRef.current = null;
      } else {
        console.error('[miniapps] UI-only Build failed', error);
        setFailure(formatError(error));
        await load('background');
      }
    } finally {
      setBuilding(false);
    }
  }, [building, canceling, load, message, miniappId, t, workshop]);

  const handleCancelBuild = useCallback(async () => {
    if (!workshop || !miniappId || canceling) return;
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
  }, [canceling, load, message, miniappId, t, workshop]);

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
              <div className={`${styles.notice} ${styles.noticeError}`}>
                {failure}
              </div>
            )}
            <MiniAppWorkshopDetail
              workshop={workshop}
              locale={i18n.language}
              onBack={goBack}
              onRefresh={() => void load('refresh')}
              onBuild={() => void handleBuild()}
              onCancelBuild={() => void handleCancelBuild()}
              refreshing={refreshing}
              building={building}
              canceling={canceling}
            />
          </>
        ) : null}
      </HubPageShell>
    </>
  );
};

export default MiniAppRunnerPage;
