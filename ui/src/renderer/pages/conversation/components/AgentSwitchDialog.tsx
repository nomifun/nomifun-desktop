/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  AgentHandoffMode,
  PreviewAgentSessionSwitchResponse,
} from '@/common/types/agentPlatform';
import { Alert, Button, Modal, Radio, Spin, Tag } from '@arco-design/web-react';
import { ArrowRight } from '@icon-park/react';
import React from 'react';
import { useTranslation } from 'react-i18next';
import styles from './AgentSwitchDialog.module.css';

export interface AgentSwitchDialogProps {
  visible: boolean;
  preview?: PreviewAgentSessionSwitchResponse;
  currentAgentLabel?: string;
  targetAgentLabel?: string;
  loading: boolean;
  applying: boolean;
  mode: AgentHandoffMode;
  error?: string;
  errorCode?: string;
  onModeChange: (mode: AgentHandoffMode) => void;
  onConfirm: () => void;
  onCancel: () => void;
  onRecovery: (code: string) => void;
}

const DiffTags: React.FC<{ values: string[]; emptyLabel: string }> = ({ values, emptyLabel }) => (
  <div className={styles.tags}>
    {values.length > 0
      ? values.map((value) => <Tag key={value} size='small'>{value}</Tag>)
      : <span className={styles.empty}>{emptyLabel}</span>}
  </div>
);

const AgentSwitchDialog: React.FC<AgentSwitchDialogProps> = ({
  visible,
  preview,
  currentAgentLabel,
  targetAgentLabel,
  loading,
  applying,
  mode,
  error,
  errorCode,
  onModeChange,
  onConfirm,
  onCancel,
  onRecovery,
}) => {
  const { t } = useTranslation();
  const blocker = preview?.blockers[0];
  const blockerMessage = blocker
    ? t(`conversation.chat.agentSwitch.blockers.${blocker.code}`, {
        defaultValue: blocker.message,
      })
    : undefined;
  const recoveryCode = blocker?.code ?? errorCode;
  const recoveryLabel = recoveryCode === 'AGENT_SESSION_MODEL_INCOMPATIBLE'
    ? t('conversation.chat.agentSwitch.openModelHub')
    : recoveryCode === 'AGENT_SESSION_RESOURCE_REQUIRED'
      ? t('conversation.chat.agentSwitch.openAgentSettings')
      : null;
  const canConfirm = Boolean(preview?.can_apply) && !loading && !applying;
  const noChanges = t('common.none', { defaultValue: 'None' });

  return (
    <Modal
      visible={visible}
      title={t('conversation.chat.agentSwitch.title')}
      onCancel={onCancel}
      unmountOnExit
      maskClosable={!applying}
      escToExit={!applying}
      style={{ width: 640 }}
      footer={(
        <div className={styles.footer}>
          <Button onClick={onCancel} disabled={applying}>{t('common.cancel')}</Button>
          <Button type='primary' loading={applying} disabled={!canConfirm} onClick={onConfirm}>
            {t('conversation.chat.agentSwitch.confirm')}
          </Button>
        </div>
      )}
    >
      <div className={styles.body} data-testid='agent-switch-dialog'>
        {loading && (
          <div className={styles.loading} role='status'>
            <Spin size={22} />
            <span>{t('conversation.chat.agentSwitch.loading')}</span>
          </div>
        )}

        {error && <Alert type='error' showIcon content={error} />}
        {recoveryCode && recoveryLabel && (
          <Button className={styles.recovery} type='text' onClick={() => onRecovery(recoveryCode)}>
            {recoveryLabel}
          </Button>
        )}

        {preview && (
          <>
            <section className={styles.transition} aria-label={t('conversation.chat.agentSwitch.title')}>
              <div className={styles.agentIdentity}>
                <span className={styles.kicker}>{t('conversation.chat.agentSwitch.currentLabel')}</span>
                <strong>{currentAgentLabel ?? preview.current.label}</strong>
                <span>v{preview.current.preset_revision}</span>
              </div>
              <div className={styles.seam} aria-hidden='true'>
                <span />
                <ArrowRight theme='outline' size={18} fill='currentColor' />
                <span />
              </div>
              <div className={styles.agentIdentity}>
                <span className={styles.kicker}>{t('conversation.chat.agentSwitch.nextTurnLabel')}</span>
                <strong>{targetAgentLabel ?? preview.target.label}</strong>
                <span>v{preview.target.preset_revision}</span>
              </div>
            </section>

            <p className={styles.nextTurn}>{t('conversation.chat.agentSwitch.nextTurn')}</p>
            <div className={styles.modelLine}>
              {t('conversation.chat.agentSwitch.modelPreserved', { model: preview.model.model })}
            </div>

            <section className={styles.diffGrid}>
              <div className={styles.diffCard}>
                <h4>{t('conversation.chat.agentSwitch.capabilityDiff')}</h4>
                <span className={styles.diffLabel}>{t('conversation.chat.agentSwitch.gained', { items: '' })}</span>
                <DiffTags values={preview.capabilities.gained} emptyLabel={noChanges} />
                <span className={styles.diffLabel}>{t('conversation.chat.agentSwitch.lost', { items: '' })}</span>
                <DiffTags values={preview.capabilities.lost} emptyLabel={noChanges} />
              </div>
              <div className={styles.diffCard}>
                <h4>{t('conversation.chat.agentSwitch.resourceDiff')}</h4>
                <p>{t('conversation.chat.agentSwitch.retainedResources', { count: preview.resources.retained.length })}</p>
                <p>{t('conversation.chat.agentSwitch.droppedResources', { count: preview.resources.dropped.length })}</p>
                {preview.resources.missing_kinds.length > 0 && (
                  <DiffTags values={preview.resources.missing_kinds} emptyLabel={noChanges} />
                )}
              </div>
            </section>

            <Radio.Group
              className={styles.modeGroup}
              value={mode}
              onChange={(value) => onModeChange(value as AgentHandoffMode)}
            >
              <div className={`${styles.modeCard} ${mode === 'continue_task' ? styles.modeCardSelected : ''}`}>
                <Radio value='continue_task' disabled={!preview.handoff.available}>
                  <span className={styles.modeCopy}>
                    <strong>{t('conversation.chat.agentSwitch.continueTask')}</strong>
                    <small>{t('conversation.chat.agentSwitch.continueTaskDescription')}</small>
                  </span>
                </Radio>
              </div>
              <div className={`${styles.modeCard} ${mode === 'context_only' ? styles.modeCardSelected : ''}`}>
                <Radio value='context_only'>
                  <span className={styles.modeCopy}>
                    <strong>{t('conversation.chat.agentSwitch.contextOnly')}</strong>
                    <small>{t('conversation.chat.agentSwitch.contextOnlyDescription')}</small>
                  </span>
                </Radio>
              </div>
            </Radio.Group>

            {!preview.handoff.available && (
              <Alert type='info' showIcon content={t('conversation.chat.agentSwitch.handoffUnavailable')} />
            )}
            {mode === 'continue_task' && preview.handoff.available && (
              <Alert type='warning' showIcon content={t('conversation.chat.agentSwitch.dataOnlyWarning')} />
            )}
            {blocker && (
              <Alert
                type='error'
                showIcon
                content={t('conversation.chat.agentSwitch.blocked', { reason: blockerMessage })}
              />
            )}
          </>
        )}
      </div>
    </Modal>
  );
};

export default AgentSwitchDialog;
