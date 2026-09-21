/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
PluginProjectDetail,
PluginSummary
} from '@/common/types/pluginPlatform';
import NomiModal from '@/renderer/components/base/NomiModal';
import { Alert,Checkbox } from '@arco-design/web-react';
import { Code } from '@icon-park/react';
import React,{ useEffect,useMemo,useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
EMPTY_TEST_INPUT_DIGEST,
type PluginApplyTargetSelection,
type PluginLoadFailure
} from './pluginWorkbenchModel';
import styles from './PluginWorkbenchPage.module.css';

export interface PluginCandidateTestModalProps {
  visible: boolean;
  detail: PluginProjectDetail | null;
  loading: boolean;
  failure?: PluginLoadFailure | null;
  onCancel: () => void;
  onSubmit: (resolvedTestInputDigest: string) => void | Promise<void>;
}

export const PluginCandidateTestModal: React.FC<PluginCandidateTestModalProps> = ({
  visible,
  loading,
  failure,
  onCancel,
  onSubmit,
}) => {
  const { t } = useTranslation();
  const [acknowledged, setAcknowledged] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!visible) return;
    setAcknowledged(false);
    setError(null);
  }, [visible]);

  const handleSubmit = async () => {
    if (!acknowledged) {
      setError(t('pluginWorkbench.dialogs.test.acknowledgementRequired'));
      return;
    }
    setError(null);
    await onSubmit(EMPTY_TEST_INPUT_DIGEST);
  };

  return (
    <NomiModal
      visible={visible}
      header={t('pluginWorkbench.dialogs.test.title')}
      onCancel={loading ? undefined : onCancel}
      onOk={() => void handleSubmit()}
      okText={t('pluginWorkbench.actions.testCandidate')}
      cancelText={t('pluginWorkbench.actions.cancel')}
      confirmLoading={loading}
      maskClosable={false}
      autoFocus={false}
      unmountOnExit
    >
      <div className={styles.dialogIntro}>{t('pluginWorkbench.dialogs.test.body')}</div>
      <div className={styles.dialogForm}>
        {failure && <Alert type='error' showIcon content={failure.message} />}
        <Alert type='warning' showIcon content={t('pluginWorkbench.dialogs.test.sideEffectWarning')} />
        <Checkbox checked={acknowledged} onChange={setAcknowledged}>
          {t('pluginWorkbench.dialogs.test.acknowledgeRisk')}
        </Checkbox>
        {error && <Alert type='error' showIcon content={error} />}
      </div>
    </NomiModal>
  );
};

export interface PluginCandidateApplyModalProps {
  visible: boolean;
  detail: PluginProjectDetail | null;
  linkedMount?: PluginSummary;
  loading: boolean;
  failure?: PluginLoadFailure | null;
  onCancel: () => void;
  onSubmit: (input: {
    target: PluginApplyTargetSelection;
    allowBreaking: boolean;
    acknowledgeTestWarning: boolean;
  }) => void | Promise<void>;
}

export const PluginCandidateApplyModal: React.FC<PluginCandidateApplyModalProps> = ({
  visible,
  detail,
  linkedMount,
  loading,
  failure,
  onCancel,
  onSubmit,
}) => {
  const { t } = useTranslation();
  const ready = detail?.ready;
  const hasLinkedMount = Boolean(detail?.summary.linked_mount_id);
  const canUseExistingMount = Boolean(hasLinkedMount && linkedMount?.current);
  const target: PluginApplyTargetSelection = hasLinkedMount ? 'existing_mount' : 'initial_install';
  const [allowBreaking, setAllowBreaking] = useState(false);
  const [acknowledgeTestWarning, setAcknowledgeTestWarning] = useState(false);

  useEffect(() => {
    if (!visible) return;
    setAllowBreaking(false);
    setAcknowledgeTestWarning(false);
  }, [visible, detail]);

  const needsBreakingAcknowledgement = ready?.impact.compatibility === 'breaking';
  const needsTestAcknowledgement = ready?.test.status !== 'passed';
  const blockedByTarget = target === 'existing_mount' && !canUseExistingMount;
  const canSubmit =
    Boolean(ready?.impact.can_apply) &&
    !blockedByTarget &&
    (!needsBreakingAcknowledgement || allowBreaking) &&
    (!needsTestAcknowledgement || acknowledgeTestWarning);

  const targetLabel = useMemo(() => {
    if (target === 'existing_mount') {
      return linkedMount?.display_name
        ? t('pluginWorkbench.dialogs.apply.existingMountNamed', {
            name: linkedMount.display_name,
          })
        : t('pluginWorkbench.dialogs.apply.existingMount');
    }
    return t('pluginWorkbench.dialogs.apply.initialInstall');
  }, [linkedMount?.display_name, t, target]);

  return (
    <NomiModal
      visible={visible}
      header={t('pluginWorkbench.dialogs.apply.title')}
      onCancel={loading ? undefined : onCancel}
      onOk={() =>
        void onSubmit({
          target,
          allowBreaking,
          acknowledgeTestWarning,
        })
      }
      okText={t('pluginWorkbench.product.saveAndEnable')}
      cancelText={t('pluginWorkbench.actions.cancel')}
      confirmLoading={loading}
      okButtonProps={{ disabled: !canSubmit }}
      maskClosable={false}
      autoFocus={false}
      unmountOnExit
    >
      <div className={styles.dialogIntro}>{t('pluginWorkbench.dialogs.apply.body')}</div>
      {!ready ? (
        <Alert type='error' showIcon content={t('pluginWorkbench.dialogs.apply.noCandidate')} />
      ) : (
        <div className={styles.dialogForm}>
          {failure && <Alert type='error' showIcon content={failure.message} />}
          <div className={styles.dialogFieldLabel}>{t('pluginWorkbench.dialogs.apply.target')}</div>
          <div className={styles.applyTarget}>
            <Code theme='outline' size='16' />
            <span>{targetLabel}</span>
          </div>
          {blockedByTarget && (
            <Alert type='error' showIcon content={t('pluginWorkbench.dialogs.apply.targetUnavailable')} />
          )}

          {needsBreakingAcknowledgement && (
            <Checkbox checked={allowBreaking} onChange={setAllowBreaking}>
              {t('pluginWorkbench.dialogs.apply.allowBreaking')}
            </Checkbox>
          )}
          {needsTestAcknowledgement && (
            <Checkbox
              checked={acknowledgeTestWarning}
              onChange={setAcknowledgeTestWarning}
            >
              {t('pluginWorkbench.dialogs.apply.acknowledgeTestWarning')}
            </Checkbox>
          )}

          {(ready.impact.changed_contracts.length > 0 || ready.impact.affected_consumers.length > 0) && (
            <details className={styles.dialogSubsection}>
              <summary>{t('common.technical_details')}</summary>
              <div className={styles.dialogChipList}>
                {ready.impact.changed_contracts.map((contract) => (
                  <span key={contract} className={`${styles.chip} ${styles.mono}`}>
                    {contract}
                  </span>
                ))}
                {ready.impact.affected_consumers.map((consumer) => (
                  <span key={`${consumer.surface}:${consumer.consumer_id}`} className={styles.chip}>
                    {consumer.surface}: {consumer.consumer_id}
                  </span>
                ))}
              </div>
            </details>
          )}
          {!ready.impact.can_apply && ready.impact.blocking_reasons.length > 0 && (
            <Alert
              type='warning'
              showIcon
              content={t('pluginWorkbench.dialogs.apply.applyBlocked')}
            />
          )}
        </div>
      )}
    </NomiModal>
  );
};
