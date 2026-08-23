/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useEffect, useRef } from 'react';

import styles from './StandaloneWorkbenchProduct.module.css';

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
}> = ({ label, error, onRetry }) => (
  <section className={styles.historyGate} role={error ? 'alert' : 'status'}>
    <strong>{error ? `${label}历史加载失败` : `正在恢复${label}历史`}</strong>
    <p>{error?.message ?? '正在读取独立工作台的任务与结果，请稍候。'}</p>
    {error ? (
      <button type='button' onClick={onRetry}>
        重试
      </button>
    ) : null}
  </section>
);

export const StandaloneHistoryRetireDialog: React.FC<{
  open: boolean;
  count: number;
  busy: boolean;
  error?: string | null;
  onCancel(): void;
  onConfirm(): void;
}> = ({ open, count, busy, error, onCancel, onConfirm }) => {
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
        <h2 id='standalone-retire-title'>从历史移除{count > 1 ? ` ${count} 条` : '这条记录'}？</h2>
        <p>
          任务审计、输入素材和生成结果会继续安全保留；这里只让所选记录不再出现在当前工作台历史中。
        </p>
        {error ? (
          <p className={styles.retireError} role='alert'>
            {error}
          </p>
        ) : null}
        <div className={styles.retireActions}>
          <button type='button' disabled={busy} onClick={onCancel}>
            取消
          </button>
          <button type='button' data-danger disabled={busy} onClick={onConfirm}>
            {busy ? '正在移除…' : '从历史移除'}
          </button>
        </div>
      </section>
    </div>
  );
};
