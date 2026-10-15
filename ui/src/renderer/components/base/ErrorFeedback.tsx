/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useId } from 'react';
import { Button } from '@arco-design/web-react';
import { useTranslation } from 'react-i18next';

import { redactSensitiveText } from '@/common/adapter/httpBridge';
import CopyIconButton from './CopyIconButton';
import NomiModal from './NomiModal';
import styles from './ErrorFeedback.module.css';

export interface ErrorFeedbackPresentation {
  readonly title: string;
  readonly message: string;
  readonly technicalDetails: string;
  readonly handled: boolean;
}

export function ErrorDetails({ technicalDetails }: { technicalDetails: string }) {
  const { t } = useTranslation();
  const safeDetails = redactSensitiveText(technicalDetails);
  if (!safeDetails.trim()) return null;
  return (
    <details className={styles.details}>
      <summary>{t('common.errorFeedback.technicalDetails', { defaultValue: '技术信息' })}</summary>
      <div className={styles.detailsContent}>
        <div className={styles.copyRow}>
          <CopyIconButton
            text={safeDetails}
            tooltip={t('common.errorFeedback.copyDetails', { defaultValue: '复制技术信息' })}
            className='h-24px w-24px'
          />
        </div>
        <pre className={styles.technicalText}>{safeDetails}</pre>
      </div>
    </details>
  );
}

export function ErrorNotice({ error, onRetry }: {
  error: ErrorFeedbackPresentation;
  onRetry?: () => void;
}) {
  const { t } = useTranslation();
  if (error.handled) return null;
  return (
    <section role='alert' className={styles.notice}>
      <div className={styles.noticeHeader}>
        <strong className={styles.title}>{error.title}</strong>
        {onRetry && <Button size='small' onClick={onRetry}>{t('common.retry', { defaultValue: '重试' })}</Button>}
      </div>
      <p className={styles.message}>{error.message}</p>
      <ErrorDetails technicalDetails={error.technicalDetails} />
    </section>
  );
}

export function ErrorModal({ error, onClose, onRetry, retryLabel }: {
  error: ErrorFeedbackPresentation | null;
  onClose: () => void;
  onRetry?: () => void;
  retryLabel?: string;
}) {
  const { t } = useTranslation();
  const descriptionId = useId();
  if (!error || error.handled) return null;

  return (
    <NomiModal
      visible
      header={error.title}
      style={{ width: 520 }}
      maskClosable={false}
      onCancel={onClose}
      modalRender={(node) => React.isValidElement<React.HTMLAttributes<HTMLDivElement>>(node)
        ? React.cloneElement(node, { 'aria-label': error.title, 'aria-describedby': descriptionId })
        : node}
      footer={
        <div className={styles.footer}>
          <Button onClick={onClose}>{t('common.close', { defaultValue: '关闭' })}</Button>
          {onRetry && <Button type='primary' onClick={() => { onClose(); onRetry(); }}>
            {retryLabel ?? t('common.retry', { defaultValue: '重试' })}
          </Button>}
        </div>
      }
    >
      <div className={styles.modalBody}>
        <p id={descriptionId} className={styles.message}>{error.message}</p>
        <ErrorDetails technicalDetails={error.technicalDetails} />
      </div>
    </NomiModal>
  );
}
