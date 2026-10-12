/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useId } from 'react';
import { Button, Message } from '@arco-design/web-react';
import type { TFunction } from 'i18next';
import { useTranslation } from 'react-i18next';
import CopyIconButton from '@/renderer/components/base/CopyIconButton';
import NomiModal from '@/renderer/components/base/NomiModal';
import { redactSensitiveText } from '@/common/adapter/httpBridge';
import { normalizeKnowledgeError, type KnowledgeErrorContext, type KnowledgeErrorPresentation } from './knowledgeError';
import styles from './KnowledgeErrorFeedback.module.css';

export function showKnowledgeErrorMessage(error: unknown, context?: KnowledgeErrorContext, t?: TFunction): void {
  const presentation = normalizeKnowledgeError(error, context, t);
  if (!presentation.handled) Message.error(presentation.message);
}

export function KnowledgeErrorDetails({ technicalDetails }: { technicalDetails: string }) {
  const { t } = useTranslation();
  const safeDetails = redactSensitiveText(technicalDetails);
  if (!safeDetails.trim()) return null;
  return (
    <details className={styles.details}>
      <summary>{t('knowledge.errors.technicalDetails', { defaultValue: '技术信息' })}</summary>
      <div className={styles.detailsContent}>
        <div className={styles.copyRow}>
          <CopyIconButton
            text={safeDetails}
            tooltip={t('knowledge.errors.copyDetails', { defaultValue: '复制技术信息' })}
            className='h-24px w-24px'
          />
        </div>
        <pre className={styles.technicalText}>{safeDetails}</pre>
      </div>
    </details>
  );
}

export function KnowledgeErrorNotice({ error, onRetry }: {
  error: KnowledgeErrorPresentation;
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
      <KnowledgeErrorDetails technicalDetails={error.technicalDetails} />
    </section>
  );
}

export function KnowledgeErrorModal({ error, onClose, onRetry, retryLabel }: {
  error: KnowledgeErrorPresentation | null;
  onClose: () => void;
  onRetry?: () => void;
  retryLabel?: string;
}) {
  const { t } = useTranslation();
  const descriptionId = useId();
  if (!error || error.handled) return null;
  const retryText = retryLabel ?? (error.context === 'export'
    ? t('knowledge.errors.chooseLocation', { defaultValue: '重新选择保存位置' })
    : t('common.retry', { defaultValue: '重试' }));

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
          {onRetry && <Button type='primary' onClick={() => { onClose(); onRetry(); }}>{retryText}</Button>}
        </div>
      }
    >
      <div className={styles.modalBody}>
        <p id={descriptionId} className={styles.message}>{error.message}</p>
        <KnowledgeErrorDetails technicalDetails={error.technicalDetails} />
      </div>
    </NomiModal>
  );
}
