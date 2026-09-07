/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import { parseMiniAppId } from '@/common/types/ids';
import type {
  MiniAppCapabilityContribution,
  MiniAppConsumerSurface,
  MiniAppOperationState,
  MiniAppReleaseRef,
  MiniAppWorkshop,
} from '@/common/types/miniAppPlatform';
import HubPageShell from '@/renderer/components/layout/HubPageShell';
import { Button } from '@arco-design/web-react';
import {
  ArrowLeft,
  CheckOne,
  CloseOne,
  Refresh,
} from '@icon-park/react';
import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate, useParams } from 'react-router-dom';
import {
  formatMiniAppTimestamp,
  miniAppPublishBlockingReasons,
  miniAppWorkflowState,
  shortMiniAppIdentity,
} from './model';
import {
  MiniAppHealthBadge,
  MiniAppKindBadge,
  MiniAppLifecycleBadge,
  MiniAppSourceBadge,
  MiniAppStatePanel,
  MiniAppTestBadge,
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

const ReleaseSlot: React.FC<{
  label: string;
  release?: MiniAppReleaseRef;
  tone: 'active' | 'ready' | 'previous';
}> = ({ label, release, tone }) => {
  const { t } = useTranslation();
  return (
    <div
      className={`${styles.releaseItem} ${
        tone === 'active'
          ? styles.releaseItemActive
          : tone === 'ready'
            ? styles.releaseItemReady
            : styles.releaseItemPrevious
      }`}
    >
      <div className={styles.releaseName}>{label}</div>
      {release ? (
        <>
          <div className={styles.releaseValue} title={release.release_id}>
            {shortMiniAppIdentity(release.release_id)}
          </div>
          <div className={styles.releaseDigest} title={release.release_digest}>
            {shortMiniAppIdentity(release.release_digest, 10)}
          </div>
        </>
      ) : (
        <div className={styles.releaseValue}>{t('miniApps.common.none')}</div>
      )}
    </div>
  );
};

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

const surfaceLabel = (
  surface: MiniAppConsumerSurface,
  t: ReturnType<typeof useTranslation>['t']
): string =>
  t(
    {
      agent: 'miniApps.workshop.capabilities.surface.agent',
      gateway: 'miniApps.workshop.capabilities.surface.gateway',
      knowledge: 'miniApps.workshop.capabilities.surface.knowledge',
      remote: 'miniApps.workshop.capabilities.surface.remote',
      automation: 'miniApps.workshop.capabilities.surface.automation',
      ui: 'miniApps.workshop.capabilities.surface.ui',
      miniapp_service: 'miniApps.workshop.capabilities.surface.miniappService',
    }[surface]
  );

const CapabilityRow: React.FC<{
  capability: MiniAppCapabilityContribution;
}> = ({ capability }) => {
  const { t } = useTranslation();
  return (
    <div className={styles.rowItem}>
      <div>
        <div className={styles.rowPrimary}>{capability.display_name}</div>
        <div
          className={`${styles.rowSecondary} ${styles.mono}`}
          title={capability.capability_id}
        >
          {capability.capability_id}
        </div>
      </div>
      <div className={styles.rowSecondary}>
        {capability.description ||
          t('miniApps.workshop.capabilities.noDescription')}
        <div className={styles.chipRow}>
          {capability.consumer_availability.map((availability) => (
            <span
              key={`${availability.surface}:${availability.status}`}
              className={styles.chip}
              title={availability.reason_code}
            >
              {surfaceLabel(availability.surface, t)}
              {' · '}
              {t(
                `miniApps.workshop.capabilities.availability.${availability.status}` as const
              )}
            </span>
          ))}
        </div>
      </div>
      <StatusBadge
        label={`v${capability.capability_version}`}
        tone='info'
      />
    </div>
  );
};

const MiniAppWorkshopDetail: React.FC<{
  workshop: MiniAppWorkshop;
  locale: string;
  onBack: () => void;
  onRefresh: () => void;
  refreshing: boolean;
}> = ({ workshop, locale, onBack, onRefresh, refreshing }) => {
  const { t } = useTranslation();
  const { miniapp, ready, active_operation: operation } = workshop;
  const workflow = miniAppWorkflowState(workshop);
  const blockingReasons = miniAppPublishBlockingReasons(workshop);
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
          icon={<Refresh theme='outline' size='14' />}
          loading={refreshing}
          onClick={onRefresh}
        >
          {t('miniApps.actions.refresh')}
        </Button>
      </div>

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
              {t('miniApps.workshop.identity.title')}
            </h3>
            <p className={styles.sectionHint}>
              {t('miniApps.workshop.identity.hint')}
            </p>
          </div>
        </div>
        <div className={styles.factGrid}>
          <div className={styles.fact}>
            <span className={styles.factLabel}>
              {t('miniApps.workshop.identity.miniappId')}
            </span>
            <span
              className={`${styles.factValue} ${styles.mono}`}
              title={miniapp.miniapp_id}
            >
              {shortMiniAppIdentity(miniapp.miniapp_id)}
            </span>
          </div>
          <div className={styles.fact}>
            <span className={styles.factLabel}>
              {t('miniApps.workshop.identity.projectId')}
            </span>
            <span
              className={`${styles.factValue} ${styles.mono}`}
              title={workshop.project_id}
            >
              {shortMiniAppIdentity(workshop.project_id)}
            </span>
          </div>
          <div className={styles.fact}>
            <span className={styles.factLabel}>
              {t('miniApps.workshop.identity.productRevision')}
            </span>
            <span className={styles.factValue}>{miniapp.product_revision}</span>
          </div>
          <div className={styles.fact}>
            <span className={styles.factLabel}>
              {t('miniApps.workshop.identity.projectRevision')}
            </span>
            <span className={styles.factValue}>{workshop.project_revision}</span>
          </div>
          <div className={styles.fact}>
            <span className={styles.factLabel}>
              {t('miniApps.workshop.identity.buildGeneration')}
            </span>
            <span className={styles.factValue}>{workshop.build_generation}</span>
          </div>
          <div className={styles.fact}>
            <span className={styles.factLabel}>
              {t('miniApps.workshop.identity.updatedAt')}
            </span>
            <span className={styles.factValue}>
              {formatMiniAppTimestamp(miniapp.updated_at_ms, locale)}
            </span>
          </div>
          <div className={styles.fact}>
            <span className={styles.factLabel}>
              {t('miniApps.workshop.identity.sourceDigest')}
            </span>
            <span
              className={`${styles.factValue} ${styles.mono}`}
              title={workshop.source_snapshot_digest}
            >
              {shortMiniAppIdentity(workshop.source_snapshot_digest)}
            </span>
          </div>
          <div className={styles.fact}>
            <span className={styles.factLabel}>
              {t('miniApps.workshop.identity.lockDigest')}
            </span>
            <span
              className={`${styles.factValue} ${styles.mono}`}
              title={workshop.dependency_lock_digest}
            >
              {shortMiniAppIdentity(workshop.dependency_lock_digest)}
            </span>
          </div>
          <div className={styles.fact}>
            <span className={styles.factLabel}>
              {t('miniApps.workshop.identity.pointerRevision')}
            </span>
            <span className={styles.factValue}>
              {miniapp.releases.pointer_revision}
            </span>
          </div>
          <div className={styles.fact}>
            <span className={styles.factLabel}>
              {t('miniApps.workshop.identity.activeEpoch')}
            </span>
            <span className={styles.factValue}>
              {miniapp.releases.active_release_epoch}
            </span>
          </div>
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
      </section>

      <section className={styles.section}>
        <div className={styles.sectionHeader}>
          <div>
            <h3 className={styles.sectionTitle}>
              {t('miniApps.workshop.releases.title')}
            </h3>
            <p className={styles.sectionHint}>
              {t('miniApps.workshop.releases.hint')}
            </p>
          </div>
        </div>
        <div className={styles.releaseGrid}>
          <ReleaseSlot
            label={t('miniApps.workshop.releases.active')}
            release={miniapp.releases.active}
            tone='active'
          />
          <ReleaseSlot
            label={t('miniApps.workshop.releases.ready')}
            release={miniapp.releases.ready}
            tone='ready'
          />
          <ReleaseSlot
            label={t('miniApps.workshop.releases.previous')}
            release={miniapp.releases.previous}
            tone='previous'
          />
        </div>
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
        </div>
        {ready ? (
          <>
            <div className={styles.factGrid}>
              <div className={styles.fact}>
                <span className={styles.factLabel}>
                  {t('miniApps.workshop.ready.releaseId')}
                </span>
                <span
                  className={`${styles.factValue} ${styles.mono}`}
                  title={ready.release.release_id}
                >
                  {shortMiniAppIdentity(ready.release.release_id)}
                </span>
              </div>
              <div className={styles.fact}>
                <span className={styles.factLabel}>
                  {t('miniApps.workshop.ready.buildGeneration')}
                </span>
                <span className={styles.factValue}>
                  {ready.project_build_generation}
                </span>
              </div>
              <div className={styles.fact}>
                <span className={styles.factLabel}>
                  {t('miniApps.workshop.ready.test')}
                </span>
                <span className={styles.factValue}>
                  <MiniAppTestBadge status={ready.test.status} />
                </span>
              </div>
              <div className={styles.fact}>
                <span className={styles.factLabel}>
                  {t('miniApps.workshop.ready.publishEligibility')}
                </span>
                <span className={styles.factValue}>
                  <StatusBadge
                    label={
                      ready.can_publish
                        ? t('miniApps.workshop.ready.canPublish')
                        : t('miniApps.workshop.ready.blocked')
                    }
                    tone={ready.can_publish ? 'success' : 'warning'}
                  />
                </span>
              </div>
              <div className={styles.fact}>
                <span className={styles.factLabel}>
                  {t('miniApps.workshop.ready.autoPublish')}
                </span>
                <span className={styles.factValue}>
                  {ready.can_auto_publish
                    ? t('miniApps.common.yes')
                    : t('miniApps.common.no')}
                </span>
              </div>
              <div className={styles.fact}>
                <span className={styles.factLabel}>
                  {t('miniApps.workshop.ready.migrations')}
                </span>
                <span className={styles.factValue}>
                  {ready.migration_count}
                </span>
              </div>
            </div>
            {blockingReasons.length > 0 && (
              <ul className={styles.blockingList}>
                {blockingReasons.map((reason) => (
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

      <section className={styles.section}>
        <div className={styles.sectionHeader}>
          <div>
            <h3 className={styles.sectionTitle}>
              {t('miniApps.workshop.configuration.title')}
            </h3>
            <p className={styles.sectionHint}>
              {t('miniApps.workshop.configuration.hint')}
            </p>
          </div>
        </div>
        <div className={styles.factGrid}>
          <div className={styles.fact}>
            <span className={styles.factLabel}>
              {t('miniApps.workshop.configuration.configRevision')}
            </span>
            <span className={styles.factValue}>
              {workshop.config.config_revision}
            </span>
          </div>
          <div className={styles.fact}>
            <span className={styles.factLabel}>
              {t('miniApps.workshop.configuration.schemaDigest')}
            </span>
            <span
              className={`${styles.factValue} ${styles.mono}`}
              title={workshop.config_schema.schema_digest}
            >
              {shortMiniAppIdentity(workshop.config_schema.schema_digest, 10)}
            </span>
          </div>
          <div className={styles.fact}>
            <span className={styles.factLabel}>
              {t('miniApps.workshop.configuration.validity')}
            </span>
            <span className={styles.factValue}>
              <StatusBadge
                label={
                  workshop.config.valid
                    ? t('miniApps.workshop.configuration.valid')
                    : t('miniApps.workshop.configuration.invalid')
                }
                tone={workshop.config.valid ? 'success' : 'danger'}
              />
            </span>
          </div>
          <div className={styles.fact}>
            <span className={styles.factLabel}>
              {t('miniApps.workshop.configuration.credentialRevision')}
            </span>
            <span className={styles.factValue}>
              {workshop.credential_bindings_revision}
            </span>
          </div>
          <div className={styles.fact}>
            <span className={styles.factLabel}>
              {t('miniApps.workshop.configuration.credentialCount')}
            </span>
            <span className={styles.factValue}>
              {workshop.credential_slots.length}
            </span>
          </div>
        </div>
        <div className={styles.jsonGrid}>
          <div>
            <div className={styles.subsectionTitle}>
              {t('miniApps.workshop.configuration.schema')}
            </div>
            <pre className={styles.jsonBlock}>
              {JSON.stringify(workshop.config_schema.schema, null, 2)}
            </pre>
          </div>
          <div>
            <div className={styles.subsectionTitle}>
              {t('miniApps.workshop.configuration.values')}
            </div>
            <pre className={styles.jsonBlock}>
              {JSON.stringify(workshop.config.values, null, 2)}
            </pre>
          </div>
        </div>
        {workshop.config.validation_errors.length > 0 && (
          <ul className={styles.blockingList}>
            {workshop.config.validation_errors.map((error) => (
              <li key={error} className={styles.blockingItem}>
                <CloseOne theme='outline' size='13' />
                <code>{error}</code>
              </li>
            ))}
          </ul>
        )}
        {workshop.credential_slots.length > 0 && (
          <div className={styles.rowList}>
            {workshop.credential_slots.map((slot) => (
              <div key={slot.slot_key} className={styles.rowItem}>
                <div>
                  <div className={styles.rowPrimary}>{slot.display_name}</div>
                  <div className={`${styles.rowSecondary} ${styles.mono}`}>
                    {slot.slot_key}
                  </div>
                </div>
                <div className={`${styles.rowSecondary} ${styles.mono}`}>
                  {slot.credential_id ||
                    t('miniApps.workshop.configuration.unbound')}
                </div>
                <StatusBadge
                  label={t(
                    `miniApps.workshop.configuration.credentialStatus.${slot.status}` as const
                  )}
                  tone={
                    slot.status === 'bound'
                      ? 'success'
                      : slot.status === 'unbound'
                        ? 'muted'
                        : 'warning'
                  }
                />
              </div>
            ))}
          </div>
        )}
      </section>

      <section className={styles.section}>
        <div className={styles.sectionHeader}>
          <div>
            <h3 className={styles.sectionTitle}>
              {t('miniApps.workshop.capabilities.title')}
            </h3>
            <p className={styles.sectionHint}>
              {t('miniApps.workshop.capabilities.hint', {
                count: workshop.capabilities.length,
              })}
            </p>
          </div>
        </div>
        {workshop.capabilities.length > 0 ? (
          <div className={styles.rowList}>
            {workshop.capabilities.map((capability) => (
              <CapabilityRow
                key={capability.provenance.contribution_id}
                capability={capability}
              />
            ))}
          </div>
        ) : (
          <MiniAppStatePanel
            title={t('miniApps.workshop.capabilities.emptyTitle')}
            body={t('miniApps.workshop.capabilities.emptyBody')}
          />
        )}
      </section>

      <section className={styles.section}>
        <div className={styles.sectionHeader}>
          <div>
            <h3 className={styles.sectionTitle}>
              {t('miniApps.workshop.service.title')}
            </h3>
            <p className={styles.sectionHint}>
              {t('miniApps.workshop.service.hint')}
            </p>
          </div>
          <MiniAppHealthBadge health={miniapp.service_health} />
        </div>
        {ready?.service ? (
          <div className={styles.serviceGrid}>
            <div className={styles.serviceItem}>
              <div className={styles.releaseName}>
                {t('miniApps.workshop.service.lifecycle')}
              </div>
              <div className={styles.releaseValue}>
                {t(
                  `miniApps.workshop.service.lifecycleValue.${
                    ready.service.lifecycle === 'on_demand'
                      ? 'onDemand'
                      : 'continuous'
                  }` as const
                )}
              </div>
            </div>
            <div className={styles.serviceItem}>
              <div className={styles.releaseName}>
                {t('miniApps.workshop.service.files')}
              </div>
              <div className={styles.releaseValue}>
                {ready.service.uses_files
                  ? t('miniApps.common.yes')
                  : t('miniApps.common.no')}
              </div>
            </div>
            <div className={styles.serviceItem}>
              <div className={styles.releaseName}>
                {t('miniApps.workshop.service.privateDatabase')}
              </div>
              <div className={styles.releaseValue}>
                {ready.service.uses_private_database
                  ? t('miniApps.common.yes')
                  : t('miniApps.common.no')}
              </div>
            </div>
          </div>
        ) : (
          <div className={styles.notice}>
            {miniapp.kind === 'ui_only'
              ? t('miniApps.workshop.service.uiOnly')
              : t('miniApps.workshop.service.noDescriptor')}
          </div>
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
                {t('miniApps.workshop.operation.kind')}
              </div>
              <div className={styles.releaseValue}>{operation.kind}</div>
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

  const [workshop, setWorkshop] = useState<MiniAppWorkshop | null>(null);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [notFound, setNotFound] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);

  const load = useCallback(
    async (mode: 'initial' | 'refresh' = 'initial') => {
      if (!miniappId) {
        setWorkshop(null);
        setNotFound(true);
        setFailure(null);
        setLoading(false);
        return;
      }
      if (mode === 'initial') setLoading(true);
      else setRefreshing(true);
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
        setRefreshing(false);
      }
    },
    [miniappId]
  );

  useEffect(() => {
    void load();
  }, [load]);

  const goBack = useCallback(() => navigate('/mini-apps'), [navigate]);

  return (
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
            refreshing={refreshing}
          />
        </>
      ) : null}
    </HubPageShell>
  );
};

export default MiniAppRunnerPage;
