/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useEffect, useRef } from 'react';
import type { TFunction } from 'i18next';
import { useTranslation } from 'react-i18next';

import {
  CreativeWorkbenchRuntimeError,
  type CreativeWorkbenchRuntimeErrorCode,
} from '../runtime';
import styles from './StandaloneWorkbenchProduct.module.css';

const WORKBENCH_ERROR_KEYS: Record<
  CreativeWorkbenchRuntimeErrorCode,
  string
> = {
  catalog_loading: 'creativeStudio.product.errors.catalogLoading',
  catalog_error: 'creativeStudio.product.errors.catalogError',
  model_required: 'creativeStudio.product.errors.modelRequired',
  model_not_compatible: 'creativeStudio.product.errors.modelNotCompatible',
  task_capability_mismatch: 'creativeStudio.product.errors.modelNotCompatible',
  invalid_parameters: 'creativeStudio.product.errors.invalidParameters',
  reference_not_owned: 'creativeStudio.product.errors.referenceInvalid',
  reference_kind_mismatch: 'creativeStudio.product.errors.referenceInvalid',
  reference_contract_mismatch: 'creativeStudio.product.errors.referenceInvalid',
  busy: 'creativeStudio.product.errors.busy',
  task_not_found: 'creativeStudio.product.errors.taskUnavailable',
  task_not_retryable: 'creativeStudio.product.errors.retryUnavailable',
  disposed: 'creativeStudio.product.errors.disposed',
  presentation_state_unsupported:
    'creativeStudio.product.errors.unsupportedState',
};

export function creativeWorkbenchErrorMessage(
  reason: unknown,
  t: TFunction
): string {
  if (reason instanceof CreativeWorkbenchRuntimeError) {
    return t(WORKBENCH_ERROR_KEYS[reason.code]);
  }
  return t('creativeStudio.product.errors.operationFailed');
}

export const StandaloneWorkbenchPage: React.FC<{
  error: string | null;
  children: React.ReactNode;
}> = ({ error, children }) => (
  <div className={styles.page} data-standalone-workbench-page>
    {error ? (
      <div className={styles.runtimeNotice} role='alert'>
        {error}
      </div>
    ) : null}
    <div className={styles.workbench}>{children}</div>
  </div>
);

export const StandaloneHistoryGate: React.FC<{
  label: string;
  error: Error | null;
  onRetry(): void;
}> = ({ label, error, onRetry }) => {
  const { t } = useTranslation();
  return (
    <section className={styles.historyGate} role={error ? 'alert' : 'status'}>
      <strong>
        {error
          ? t('creativeStudio.product.history.loadFailed', {
              defaultValue: '{{label}}历史加载失败',
              label,
            })
          : t('creativeStudio.product.history.restoring', {
              defaultValue: '正在恢复{{label}}历史',
              label,
            })}
      </strong>
      <p>
        {error
          ? creativeWorkbenchErrorMessage(error, t)
          : t('creativeStudio.product.history.loadingDescription', {
              defaultValue: '正在读取独立工作台的任务与结果，请稍候。',
            })}
      </p>
      {error ? (
        <button type='button' onClick={onRetry}>
          {t('creativeStudio.product.history.retry', { defaultValue: '重试' })}
        </button>
      ) : null}
    </section>
  );
};

export const StandaloneHistoryRetireDialog: React.FC<{
  open: boolean;
  count: number;
  busy: boolean;
  error?: string | null;
  onCancel(): void;
  onConfirm(): void;
}> = ({ open, count, busy, error, onCancel, onConfirm }) => {
  const { t } = useTranslation();
  const dialogRef = useRef<HTMLElement | null>(null);
  useEffect(() => {
    if (open) dialogRef.current?.focus();
  }, [open]);
  if (!open) return null;
  return (
    <div
      className={styles.retireBackdrop}
      onMouseDown={(event) => {
        if (!busy && event.target === event.currentTarget) onCancel();
      }}
    >
      <section
        ref={dialogRef}
        className={styles.retireDialog}
        role='dialog'
        aria-modal='true'
        aria-labelledby='standalone-retire-title'
        tabIndex={-1}
        onKeyDown={(event) => {
          if (event.key === 'Escape' && !busy) onCancel();
        }}
      >
        <h2 id='standalone-retire-title'>
          {count > 1
            ? t('creativeStudio.product.history.retireManyTitle', {
                defaultValue: '从历史移除 {{recordCount}} 条？',
                recordCount: count,
              })
            : t('creativeStudio.product.history.retireOneTitle', {
                defaultValue: '从历史移除这条记录？',
              })}
        </h2>
        <p>
          {t('creativeStudio.product.history.retireDescription', {
            defaultValue:
              '任务审计、输入素材和生成结果会继续安全保留；这里只让所选记录不再出现在当前工作台历史中。',
          })}
        </p>
        {error ? (
          <p className={styles.retireError} role='alert'>
            {error}
          </p>
        ) : null}
        <div className={styles.retireActions}>
          <button type='button' disabled={busy} onClick={onCancel}>
            {t('creativeStudio.product.history.cancel', { defaultValue: '取消' })}
          </button>
          <button type='button' data-danger disabled={busy} onClick={onConfirm}>
            {busy
              ? t('creativeStudio.product.history.retiring', { defaultValue: '正在移除…' })
              : t('creativeStudio.product.history.retire', { defaultValue: '从历史移除' })}
          </button>
        </div>
      </section>
    </div>
  );
};
