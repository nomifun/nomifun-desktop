/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  MiniAppKind,
  MiniAppLifecycle,
  MiniAppProjectSourceState,
  MiniAppServiceHealth,
  MiniAppTestStatus,
} from '@/common/types/miniAppPlatform';
import { Attention, CheckOne, CloseOne, CloudStorage, Code, PlayOne } from '@icon-park/react';
import { Button, Spin } from '@arco-design/web-react';
import React from 'react';
import { useTranslation } from 'react-i18next';
import type { I18nKey } from '@/renderer/services/i18n/i18n-keys';
import styles from './MiniAppWorkbench.module.css';

export type BadgeTone = 'success' | 'muted' | 'warning' | 'danger' | 'info';

const toneClass: Record<BadgeTone, string> = {
  success: styles.statusSuccess,
  muted: styles.statusMuted,
  warning: styles.statusWarning,
  danger: styles.statusDanger,
  info: styles.statusInfo,
};

export const StatusBadge: React.FC<{
  label: string;
  tone: BadgeTone;
}> = ({ label, tone }) => (
  <span className={`${styles.statusBadge} ${toneClass[tone]}`}>{label}</span>
);

export const MiniAppKindBadge: React.FC<{ kind: MiniAppKind }> = ({ kind }) => {
  const { t } = useTranslation();
  return (
    <StatusBadge
      label={t(`miniApps.library.kind.${kind === 'ui_only' ? 'uiOnly' : 'service'}` as const)}
      tone={kind === 'ui_only' ? 'info' : 'success'}
    />
  );
};

export const MiniAppLifecycleBadge: React.FC<{
  lifecycle: MiniAppLifecycle;
}> = ({ lifecycle }) => {
  const { t } = useTranslation();
  const labelKey: Record<MiniAppLifecycle, I18nKey> = {
    enabled: 'miniApps.library.lifecycle.enabled',
    disabled: 'miniApps.library.lifecycle.disabled',
    trashed: 'miniApps.library.lifecycle.trashed',
    deleting: 'miniApps.library.lifecycle.deleting',
  };
  const tone: Record<MiniAppLifecycle, BadgeTone> = {
    enabled: 'success',
    disabled: 'muted',
    trashed: 'warning',
    deleting: 'danger',
  };
  return <StatusBadge label={t(labelKey[lifecycle])} tone={tone[lifecycle]} />;
};

export const MiniAppSourceBadge: React.FC<{
  source: MiniAppProjectSourceState;
}> = ({ source }) => {
  const { t } = useTranslation();
  const labelKey: Record<MiniAppProjectSourceState, I18nKey> = {
    empty: 'miniApps.workshop.source.state.empty',
    editable: 'miniApps.workshop.source.state.editable',
    runtime_only: 'miniApps.workshop.source.state.runtimeOnly',
  };
  const tone: Record<MiniAppProjectSourceState, BadgeTone> = {
    empty: 'muted',
    editable: 'success',
    runtime_only: 'info',
  };
  return <StatusBadge label={t(labelKey[source])} tone={tone[source]} />;
};

export const MiniAppTestBadge: React.FC<{ status: MiniAppTestStatus }> = ({
  status,
}) => {
  const { t } = useTranslation();
  const labelKey: Record<MiniAppTestStatus, I18nKey> = {
    not_required: 'miniApps.workshop.ready.testStatus.notRequired',
    not_run: 'miniApps.workshop.ready.testStatus.notRun',
    passed: 'miniApps.workshop.ready.testStatus.passed',
    failed: 'miniApps.workshop.ready.testStatus.failed',
    needs_test_input: 'miniApps.workshop.ready.testStatus.needsInput',
    stale: 'miniApps.workshop.ready.testStatus.stale',
  };
  const tone: Record<MiniAppTestStatus, BadgeTone> = {
    not_required: 'muted',
    not_run: 'muted',
    passed: 'success',
    failed: 'danger',
    needs_test_input: 'warning',
    stale: 'warning',
  };
  return <StatusBadge label={t(labelKey[status])} tone={tone[status]} />;
};

export const MiniAppHealthBadge: React.FC<{
  health: MiniAppServiceHealth;
}> = ({ health }) => {
  const { t } = useTranslation();
  const state = health.state;
  const labelKey: Record<MiniAppServiceHealth['state'], I18nKey> = {
    not_applicable: 'miniApps.workshop.service.health.notApplicable',
    stopped: 'miniApps.workshop.service.health.stopped',
    starting: 'miniApps.workshop.service.health.starting',
    ready: 'miniApps.workshop.service.health.ready',
    failed: 'miniApps.workshop.service.health.failed',
  };
  const tone: Record<MiniAppServiceHealth['state'], BadgeTone> = {
    not_applicable: 'muted',
    stopped: 'muted',
    starting: 'info',
    ready: 'success',
    failed: 'danger',
  };
  return <StatusBadge label={t(labelKey[state])} tone={tone[state]} />;
};

export const MiniAppStatePanel: React.FC<{
  loading?: boolean;
  title: string;
  body: string;
  onRetry?: () => void;
}> = ({ loading = false, title, body, onRetry }) => {
  const { t } = useTranslation();
  if (loading) {
    return (
      <div className={styles.statePanel}>
        <Spin size={24} />
        <span className={styles.stateBody}>{body}</span>
      </div>
    );
  }
  return (
    <div className={styles.statePanel}>
      <span className={styles.stateIcon}>
        <Attention theme='outline' size='24' />
      </span>
      <span className={styles.stateTitle}>{title}</span>
      <span className={styles.stateBody}>{body}</span>
      {onRetry && (
        <Button
          size='small'
          icon={<PlayOne theme='outline' size='14' />}
          onClick={onRetry}
        >
          {t('miniApps.actions.retry')}
        </Button>
      )}
    </div>
  );
};

export const WorkflowIcon: React.FC<{
  state: 'done' | 'active' | 'blocked' | 'pending';
}> = ({ state }) => {
  if (state === 'done') return <CheckOne theme='outline' size='13' />;
  if (state === 'blocked') return <CloseOne theme='outline' size='13' />;
  if (state === 'active') return <Code theme='outline' size='13' />;
  return <CloudStorage theme='outline' size='13' />;
};
