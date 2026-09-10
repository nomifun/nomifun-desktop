/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  DurablePluginOperationState,
  PluginMountId,
  PluginProjectDetail,
  PluginProjectId,
  PluginProjectSummary,
} from '@/common/types/pluginPlatform';
import { Button, Tooltip } from '@arco-design/web-react';
import { CheckOne, CloseOne, Code, Delete, PlayOne, Plug } from '@icon-park/react';
import React from 'react';
import { useTranslation } from 'react-i18next';
import type { PluginLoadFailure } from './pluginWorkbenchModel';
import {
  formatPluginTimestamp,
  projectDeleteAvailable,
  shortPluginIdentity,
} from './pluginWorkbenchModel';
import styles from './PluginWorkbenchPage.module.css';
import {
  PluginCompatibilityBadge,
  PluginSourceBadge,
  PluginStatePanel,
  PluginTestBadge,
  StatusBadge,
} from './PluginWorkbenchState';

export type PluginProjectBusyAction =
  | 'create'
  | 'import'
  | 'edit'
  | 'dependencies'
  | 'build'
  | 'test'
  | 'apply'
  | 'delete'
  | 'cancel_operation'
  | null;

interface PluginWorkshopViewProps {
  projects: PluginProjectSummary[];
  hasQuery: boolean;
  selectedProjectId: PluginProjectId | null;
  detail?: PluginProjectDetail;
  detailLoading: boolean;
  detailFailure: PluginLoadFailure | null;
  mutationFailure: PluginLoadFailure | null;
  busyAction: PluginProjectBusyAction;
  locale: string;
  onSelect: (projectId: PluginProjectId) => void;
  onRetryDetail: () => void;
  onOpenMount: (mountId: PluginMountId) => void;
  onBuild: () => void;
  onEditSource: () => void;
  onEditDependencies: () => void;
  onTest: () => void;
  onApply: () => void;
  onDelete: () => void;
  onCancelOperation: () => void;
}

const OperationBadge: React.FC<{ state: DurablePluginOperationState }> = ({
  state,
}) => {
  const { t } = useTranslation();
  const labels: Record<DurablePluginOperationState, string> = {
    running: t('pluginWorkbench.operation.running'),
    succeeded: t('pluginWorkbench.operation.succeeded'),
    failed: t('pluginWorkbench.operation.failed'),
    canceled: t('pluginWorkbench.operation.canceled'),
  };
  const tones: Record<
    DurablePluginOperationState,
    'success' | 'info' | 'danger' | 'muted'
  > = {
    running: 'info',
    succeeded: 'success',
    failed: 'danger',
    canceled: 'muted',
  };
  return <StatusBadge label={labels[state]} tone={tones[state]} />;
};

const PluginProjectDetailPanel: React.FC<{
  detail: PluginProjectDetail;
  mutationFailure: PluginLoadFailure | null;
  busyAction: PluginProjectBusyAction;
  locale: string;
  onOpenMount: (mountId: PluginMountId) => void;
  onBuild: () => void;
  onEditSource: () => void;
  onEditDependencies: () => void;
  onTest: () => void;
  onApply: () => void;
  onDelete: () => void;
  onCancelOperation: () => void;
}> = ({
  detail,
  mutationFailure,
  busyAction,
  locale,
  onOpenMount,
  onBuild,
  onEditSource,
  onEditDependencies,
  onTest,
  onApply,
  onDelete,
  onCancelOperation,
}) => {
  const { t } = useTranslation();
  const { summary, ready, active_operation: operation } = detail;
  const canDelete = projectDeleteAvailable(detail);
  const operationRunning = operation?.state === 'running';
  const canBuild =
    summary.source_state === 'editable' &&
    Boolean(detail.source_snapshot_digest) &&
    Boolean(detail.dependency_lock_digest) &&
    !operationRunning;
  const canTest = Boolean(ready) && !operationRunning;
  const canApply = Boolean(ready?.impact.can_apply) && !operationRunning;
  const disabled = busyAction !== null;
  const workflowSteps = [
    {
      key: 'source',
      label: t('pluginWorkbench.workflow.source'),
      state:
        summary.source_state === 'empty'
          ? 'pending'
          : 'done',
    },
    {
      key: 'candidate',
      label: t('pluginWorkbench.workflow.candidate'),
      state: operationRunning ? 'active' : ready ? 'done' : 'pending',
    },
    {
      key: 'test',
      label: t('pluginWorkbench.workflow.test'),
      state:
        ready?.test.status === 'passed'
          ? 'done'
          : ready?.test.status === 'failed'
            ? 'blocked'
            : ready
              ? 'active'
              : 'pending',
    },
    {
      key: 'apply',
      label: t('pluginWorkbench.workflow.apply'),
      state: summary.linked_mount_id && !ready ? 'done' : ready ? 'active' : 'pending',
    },
  ] as const;

  return (
    <>
      <header className={styles.detailHeader}>
        <div className={styles.detailHeaderCopy}>
          <span className={styles.eyebrow}>{t('pluginWorkbench.workshop.detailEyebrow')}</span>
          <div className={styles.detailTitleRow}>
            <h2 className={styles.detailTitle}>{summary.display_name}</h2>
            <PluginSourceBadge source={summary.source_state} />
            {summary.ready_candidate && (
              <StatusBadge label={t('pluginWorkbench.workshop.ready')} tone='info' />
            )}
          </div>
          <p className={styles.detailDescription}>
            {summary.description || t('pluginWorkbench.workshop.projectDescription')}
          </p>
        </div>
      </header>

      <ol className={styles.workflow} aria-label={t('pluginWorkbench.workflow.ariaLabel')}>
        {workflowSteps.map((step, index) => (
          <li
            key={step.key}
            className={`${styles.workflowStep} ${styles[`workflowStep_${step.state}`]}`}
          >
            <span className={styles.workflowIndex}>
              {step.state === 'done' ? <CheckOne theme='outline' size='13' /> : index + 1}
            </span>
            <span>{step.label}</span>
          </li>
        ))}
      </ol>

      {mutationFailure && (
        <div className={`${styles.notice} ${styles.noticeError}`}>
          <span>{t('pluginWorkbench.states.mutationErrorTitle')}</span>
          <span>{mutationFailure.message}</span>
        </div>
      )}

      <div className={styles.actionBar}>
        {summary.linked_mount_id && (
          <Button
            icon={<Plug theme='outline' size='14' />}
            disabled={disabled}
            onClick={() => onOpenMount(summary.linked_mount_id!)}
          >
            {t('pluginWorkbench.actions.openInstalled')}
          </Button>
        )}
        <Tooltip
          content={
            canBuild
              ? ''
              : summary.source_state === 'runtime_only'
                ? t('pluginWorkbench.workshop.runtimeOnlyBuild')
                : t('pluginWorkbench.workshop.buildUnavailable')
          }
          disabled={canBuild}
        >
          <span>
            <Button
              icon={<Code theme='outline' size='14' />}
              loading={busyAction === 'build'}
              disabled={disabled || !canBuild}
              onClick={onBuild}
            >
              {t('pluginWorkbench.actions.build')}
            </Button>
          </span>
        </Tooltip>
        {canBuild && (
          <Button
            icon={<Code theme='outline' size='14' />}
            disabled={disabled}
            onClick={onEditSource}
          >
            {t('pluginWorkbench.actions.editSource')}
          </Button>
        )}
        {canBuild && (
          <Button
            icon={<Code theme='outline' size='14' />}
            loading={busyAction === 'dependencies'}
            disabled={disabled}
            onClick={onEditDependencies}
          >
            {t('pluginWorkbench.actions.editDependencies')}
          </Button>
        )}
        {ready && (
          <Button
            icon={<PlayOne theme='outline' size='14' />}
            loading={busyAction === 'test'}
            disabled={disabled || !canTest}
            onClick={onTest}
          >
            {t('pluginWorkbench.actions.testCandidate')}
          </Button>
        )}
        {ready && (
          <Button
            type='primary'
            icon={<CheckOne theme='outline' size='14' />}
            loading={busyAction === 'apply'}
            disabled={disabled || !canApply}
            onClick={onApply}
          >
            {t('pluginWorkbench.actions.applyCandidate')}
          </Button>
        )}
        {operation?.cancelable && operationRunning && (
          <Button
            icon={<CloseOne theme='outline' size='14' />}
            loading={busyAction === 'cancel_operation'}
            disabled={disabled}
            onClick={onCancelOperation}
          >
            {t('pluginWorkbench.actions.cancelOperation')}
          </Button>
        )}
        <Button
          status='danger'
          icon={<Delete theme='outline' size='14' />}
          loading={busyAction === 'delete'}
          disabled={disabled || !canDelete}
          onClick={onDelete}
        >
          {t('pluginWorkbench.actions.deleteProject')}
        </Button>
      </div>

      <section className={styles.detailSection}>
        <div className={styles.sectionHeader}>
          <div>
            <h3 className={styles.sectionTitle}>{t('pluginWorkbench.detail.identity')}</h3>
            <p className={styles.sectionHint}>{t('pluginWorkbench.workshop.identityHint')}</p>
          </div>
        </div>
        <div className={styles.factGrid}>
          <div className={styles.fact}>
            <span className={styles.factLabel}>{t('pluginWorkbench.detail.projectId')}</span>
            <span className={`${styles.factValue} ${styles.mono}`} title={summary.project_id}>
              {shortPluginIdentity(summary.project_id)}
            </span>
          </div>
          <div className={styles.fact}>
            <span className={styles.factLabel}>{t('pluginWorkbench.detail.revision')}</span>
            <span className={styles.factValue}>{summary.project_revision}</span>
          </div>
          <div className={styles.fact}>
            <span className={styles.factLabel}>{t('pluginWorkbench.detail.buildGeneration')}</span>
            <span className={styles.factValue}>{summary.build_generation}</span>
          </div>
          <div className={styles.fact}>
            <span className={styles.factLabel}>{t('pluginWorkbench.detail.linkedMount')}</span>
            <span className={`${styles.factValue} ${styles.mono}`} title={summary.linked_mount_id}>
              {shortPluginIdentity(summary.linked_mount_id)}
            </span>
          </div>
          <div className={styles.fact}>
            <span className={styles.factLabel}>{t('pluginWorkbench.detail.applyMode')}</span>
            <span className={styles.factValue}>
              {summary.apply_mode === 'auto_compatible_when_idle'
                ? t('pluginWorkbench.applyMode.autoCompatible')
                : t('pluginWorkbench.applyMode.ask')}
            </span>
          </div>
          <div className={styles.fact}>
            <span className={styles.factLabel}>{t('pluginWorkbench.detail.updatedAt')}</span>
            <span className={styles.factValue}>
              {formatPluginTimestamp(summary.updated_at_ms, locale)}
            </span>
          </div>
          <div className={styles.fact}>
            <span className={styles.factLabel}>{t('pluginWorkbench.detail.sourceDigest')}</span>
            <span
              className={`${styles.factValue} ${styles.mono}`}
              title={detail.source_snapshot_digest}
            >
              {shortPluginIdentity(detail.source_snapshot_digest)}
            </span>
          </div>
          <div className={styles.fact}>
            <span className={styles.factLabel}>{t('pluginWorkbench.detail.lockDigest')}</span>
            <span
              className={`${styles.factValue} ${styles.mono}`}
              title={detail.dependency_lock_digest}
            >
              {shortPluginIdentity(detail.dependency_lock_digest)}
            </span>
          </div>
        </div>
        {summary.source_state === 'runtime_only' && (
          <div className={`${styles.notice} ${styles.noticeWarning} ${styles.noticeInSection}`}>
            {t('pluginWorkbench.workshop.runtimeOnlyNotice')}
          </div>
        )}
      </section>

      <section className={styles.detailSection}>
        <div className={styles.sectionHeader}>
          <div>
            <h3 className={styles.sectionTitle}>{t('pluginWorkbench.workshop.readyCandidate')}</h3>
            <p className={styles.sectionHint}>{t('pluginWorkbench.workshop.readyCandidateHint')}</p>
          </div>
        </div>
        {ready ? (
          <>
            <div className={styles.candidateSummary}>
              <div className={styles.fact}>
                <span className={styles.factLabel}>{t('pluginWorkbench.detail.targetVersion')}</span>
                <span className={styles.factValue}>v{ready.target.package_version}</span>
              </div>
              <div className={styles.fact}>
                <span className={styles.factLabel}>{t('pluginWorkbench.detail.compatibility')}</span>
                <span className={styles.factValue}>
                  <PluginCompatibilityBadge compatibility={ready.impact.compatibility} />
                </span>
              </div>
              <div className={styles.fact}>
                <span className={styles.factLabel}>{t('pluginWorkbench.detail.test')}</span>
                <span className={styles.factValue}>
                  <PluginTestBadge status={ready.test.status} />
                </span>
              </div>
              <div className={styles.fact}>
                <span className={styles.factLabel}>{t('pluginWorkbench.detail.origin')}</span>
                <span className={styles.factValue}>
                  {ready.origin === 'build'
                    ? t('pluginWorkbench.origin.build')
                    : t('pluginWorkbench.origin.import')}
                </span>
              </div>
              <div className={styles.fact}>
                <span className={styles.factLabel}>{t('pluginWorkbench.detail.candidateId')}</span>
                <span
                  className={`${styles.factValue} ${styles.mono}`}
                  title={ready.candidate.candidate_id}
                >
                  {shortPluginIdentity(ready.candidate.candidate_id)}
                </span>
              </div>
              <div className={styles.fact}>
                <span className={styles.factLabel}>{t('pluginWorkbench.detail.applyEligibility')}</span>
                <span className={styles.factValue}>
                  {ready.impact.can_apply
                    ? t('pluginWorkbench.workshop.canApply')
                    : t('pluginWorkbench.workshop.blocked')}
                </span>
              </div>
            </div>
            {ready.impact.blocking_reasons.length > 0 && (
              <ul className={styles.blockingList}>
                {ready.impact.blocking_reasons.map((reason) => (
                  <li key={reason}>{reason}</li>
                ))}
              </ul>
            )}
          </>
        ) : (
          <PluginStatePanel
            compact
            title={t('pluginWorkbench.workshop.noCandidate')}
            body={t('pluginWorkbench.workshop.noCandidateBody')}
          />
        )}
      </section>

      <section className={styles.detailSection}>
        <div className={styles.sectionHeader}>
          <div>
            <h3 className={styles.sectionTitle}>{t('pluginWorkbench.workshop.operation')}</h3>
            <p className={styles.sectionHint}>{t('pluginWorkbench.workshop.operationHint')}</p>
          </div>
        </div>
        {operation ? (
          <div className={styles.operationSummary}>
            <div className={styles.fact}>
              <span className={styles.factLabel}>{t('pluginWorkbench.detail.operationState')}</span>
              <span className={styles.factValue}>
                <OperationBadge state={operation.state} />
              </span>
            </div>
            <div className={styles.fact}>
              <span className={styles.factLabel}>{t('pluginWorkbench.detail.operationKind')}</span>
              <span className={styles.factValue}>{operation.kind}</span>
            </div>
            <div className={styles.fact}>
              <span className={styles.factLabel}>{t('pluginWorkbench.detail.progress')}</span>
              <span className={styles.factValue}>
                {operation.progress_percent == null
                  ? t('pluginWorkbench.common.unknown')
                  : `${operation.progress_percent}%`}
              </span>
            </div>
          </div>
        ) : (
          <PluginStatePanel
            compact
            title={t('pluginWorkbench.workshop.noOperation')}
            body={t('pluginWorkbench.workshop.noOperationBody')}
          />
        )}
      </section>
    </>
  );
};

const PluginWorkshopView: React.FC<PluginWorkshopViewProps> = ({
  projects,
  hasQuery,
  selectedProjectId,
  detail,
  detailLoading,
  detailFailure,
  mutationFailure,
  busyAction,
  locale,
  onSelect,
  onRetryDetail,
  onOpenMount,
  onBuild,
  onEditSource,
  onEditDependencies,
  onTest,
  onApply,
  onDelete,
  onCancelOperation,
}) => {
  const { t } = useTranslation();

  return (
    <div className={styles.workspace}>
      <aside className={styles.collection} aria-label={t('pluginWorkbench.workshop.ariaLabel')}>
        <div className={styles.collectionHeader}>
          <span className={styles.collectionTitle}>{t('pluginWorkbench.workshop.title')}</span>
          <span className={styles.collectionCount}>
            {t('pluginWorkbench.common.count', { count: projects.length })}
          </span>
        </div>
        {projects.length === 0 ? (
          <PluginStatePanel
            compact
            title={
              hasQuery
                ? t('pluginWorkbench.workshop.noMatchesTitle')
                : t('pluginWorkbench.workshop.emptyTitle')
            }
            body={
              hasQuery
                ? t('pluginWorkbench.workshop.noMatchesBody')
                : t('pluginWorkbench.workshop.emptyBody')
            }
          />
        ) : (
          <div className={styles.list}>
            {projects.map((project) => (
              <button
                key={project.project_id}
                type='button'
                className={`${styles.listRow} ${
                  selectedProjectId === project.project_id ? styles.listRowActive : ''
                }`}
                onClick={() => onSelect(project.project_id)}
              >
                <span className={styles.listIcon}>
                  <Code theme='outline' size='17' />
                </span>
                <span className={styles.listCopy}>
                  <span className={styles.listName}>{project.display_name}</span>
                  <span className={styles.listMeta}>
                    {t('pluginWorkbench.workshop.generation', {
                      generation: project.build_generation,
                    })}
                    {project.ready_candidate
                      ? ` · ${t('pluginWorkbench.workshop.ready')}`
                      : ` · ${
                          project.linked_mount_id
                            ? t('pluginWorkbench.workshop.linked')
                            : t('pluginWorkbench.workshop.unlinkedDraft')
                        }`}
                  </span>
                </span>
                <PluginSourceBadge source={project.source_state} />
              </button>
            ))}
          </div>
        )}
      </aside>

      <main className={styles.detail}>
        {!selectedProjectId ? (
          <PluginStatePanel
            title={t('pluginWorkbench.workshop.selectTitle')}
            body={t('pluginWorkbench.workshop.selectBody')}
          />
        ) : detailLoading && !detail ? (
          <PluginStatePanel loading body={t('pluginWorkbench.states.loadingProject')} />
        ) : detailFailure ? (
          <PluginStatePanel failure={detailFailure} onRetry={onRetryDetail} />
        ) : detail ? (
          <PluginProjectDetailPanel
            detail={detail}
            mutationFailure={mutationFailure}
            busyAction={busyAction}
            locale={locale}
            onOpenMount={onOpenMount}
            onBuild={onBuild}
            onEditSource={onEditSource}
            onEditDependencies={onEditDependencies}
            onTest={onTest}
            onApply={onApply}
            onDelete={onDelete}
            onCancelOperation={onCancelOperation}
          />
        ) : (
          <PluginStatePanel
            title={t('pluginWorkbench.workshop.selectTitle')}
            body={t('pluginWorkbench.workshop.selectBody')}
          />
        )}
      </main>
    </div>
  );
};

export default PluginWorkshopView;
