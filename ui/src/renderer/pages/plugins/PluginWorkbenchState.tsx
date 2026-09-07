/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  PluginCandidateTestStatus,
  PluginCompatibility,
  PluginLifecycle,
  PluginProjectSourceState,
} from '@/common/types/pluginPlatform';
import { Button, Spin } from '@arco-design/web-react';
import { Attention, CloseOne, Plug, Refresh } from '@icon-park/react';
import React from 'react';
import { useTranslation } from 'react-i18next';
import type { PluginLoadFailure } from './pluginWorkbenchModel';
import styles from './PluginWorkbenchPage.module.css';

type BadgeTone = 'success' | 'muted' | 'warning' | 'danger' | 'info';

const toneClass: Record<BadgeTone, string> = {
  success: styles.statusSuccess,
  muted: styles.statusMuted,
  warning: styles.statusWarning,
  danger: styles.statusDanger,
  info: styles.statusInfo,
};

export const StatusBadge: React.FC<{ label: string; tone: BadgeTone }> = ({
  label,
  tone,
}) => (
  <span className={`${styles.statusBadge} ${toneClass[tone]}`}>{label}</span>
);

export const PluginLifecycleBadge: React.FC<{ lifecycle: PluginLifecycle }> = ({
  lifecycle,
}) => {
  const { t } = useTranslation();
  const labels: Record<PluginLifecycle, string> = {
    enabled: t('pluginWorkbench.lifecycle.enabled'),
    disabled: t('pluginWorkbench.lifecycle.disabled'),
    uninstalled_data_retained: t('pluginWorkbench.lifecycle.uninstalledDataRetained'),
    delete_pending: t('pluginWorkbench.lifecycle.deletePending'),
    error: t('pluginWorkbench.lifecycle.error'),
  };
  const tones: Record<PluginLifecycle, BadgeTone> = {
    enabled: 'success',
    disabled: 'muted',
    uninstalled_data_retained: 'warning',
    delete_pending: 'warning',
    error: 'danger',
  };
  return <StatusBadge label={labels[lifecycle]} tone={tones[lifecycle]} />;
};

export const PluginSourceBadge: React.FC<{ source: PluginProjectSourceState }> = ({
  source,
}) => {
  const { t } = useTranslation();
  const labels: Record<PluginProjectSourceState, string> = {
    empty: t('pluginWorkbench.sourceState.empty'),
    editable: t('pluginWorkbench.sourceState.editable'),
    runtime_only: t('pluginWorkbench.sourceState.runtimeOnly'),
  };
  const tones: Record<PluginProjectSourceState, BadgeTone> = {
    empty: 'muted',
    editable: 'success',
    runtime_only: 'info',
  };
  return <StatusBadge label={labels[source]} tone={tones[source]} />;
};

export const PluginCompatibilityBadge: React.FC<{
  compatibility: PluginCompatibility;
}> = ({ compatibility }) => {
  const { t } = useTranslation();
  const labels: Record<PluginCompatibility, string> = {
    compatible: t('pluginWorkbench.compatibility.compatible'),
    breaking: t('pluginWorkbench.compatibility.breaking'),
    unknown: t('pluginWorkbench.compatibility.unknown'),
  };
  const tones: Record<PluginCompatibility, BadgeTone> = {
    compatible: 'success',
    breaking: 'danger',
    unknown: 'warning',
  };
  return <StatusBadge label={labels[compatibility]} tone={tones[compatibility]} />;
};

export const PluginTestBadge: React.FC<{ status: PluginCandidateTestStatus }> = ({
  status,
}) => {
  const { t } = useTranslation();
  const labels: Record<PluginCandidateTestStatus, string> = {
    not_run: t('pluginWorkbench.testStatus.notRun'),
    passed: t('pluginWorkbench.testStatus.passed'),
    failed: t('pluginWorkbench.testStatus.failed'),
    needs_test_input: t('pluginWorkbench.testStatus.needsInput'),
    stale: t('pluginWorkbench.testStatus.stale'),
  };
  const tones: Record<PluginCandidateTestStatus, BadgeTone> = {
    not_run: 'muted',
    passed: 'success',
    failed: 'danger',
    needs_test_input: 'warning',
    stale: 'warning',
  };
  return <StatusBadge label={labels[status]} tone={tones[status]} />;
};

interface PluginStatePanelProps {
  loading?: boolean;
  failure?: PluginLoadFailure | null;
  title?: string;
  body?: string;
  onRetry?: () => void;
  compact?: boolean;
}

export const PluginStatePanel: React.FC<PluginStatePanelProps> = ({
  loading = false,
  failure,
  title,
  body,
  onRetry,
  compact = false,
}) => {
  const { t } = useTranslation();

  if (loading) {
    return (
      <div className={compact ? styles.emptyList : styles.statePanel}>
        <Spin size={compact ? 18 : 24} />
        <span className={styles.stateBody}>
          {body ?? t('pluginWorkbench.states.loadingDetail')}
        </span>
      </div>
    );
  }

  const unavailable = failure?.kind === 'unavailable';
  const resolvedTitle =
    title ??
    (unavailable
      ? t('pluginWorkbench.states.unavailableTitle')
      : t('pluginWorkbench.states.errorTitle'));
  const resolvedBody =
    body ??
    failure?.message ??
    (unavailable
      ? t('pluginWorkbench.states.unavailableBody')
      : t('pluginWorkbench.states.errorBody'));

  return (
    <div className={compact ? styles.emptyList : styles.statePanel}>
      <span className={styles.stateIcon}>
        {unavailable ? (
          <Plug theme='outline' size='24' />
        ) : failure ? (
          <CloseOne theme='outline' size='24' />
        ) : (
          <Attention theme='outline' size='24' />
        )}
      </span>
      <span className={styles.stateTitle}>{resolvedTitle}</span>
      <span className={styles.stateBody}>{resolvedBody}</span>
      {onRetry && (
        <Button
          size='small'
          icon={<Refresh theme='outline' size='14' />}
          onClick={onRetry}
        >
          {t('pluginWorkbench.actions.retry')}
        </Button>
      )}
    </div>
  );
};
