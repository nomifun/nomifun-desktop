/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { CheckOne, Close, Error, Loading, Refresh } from '@icon-park/react';
import React from 'react';

import type { CreativeAssetLibraryLabels, CreativeAssetUploadItem } from './types';
import styles from './CreativeAssetLibrary.module.css';

export interface CreativeAssetUploadQueueProps {
  items: readonly CreativeAssetUploadItem[];
  labels: CreativeAssetLibraryLabels;
  onCancel?: (uploadId: string) => void;
  onRetry?: (uploadId: string) => void;
  onDismiss?: (uploadId: string) => void;
}

const clampPercent = (value: number) => Math.max(0, Math.min(100, Number.isFinite(value) ? value : 0));

const CreativeAssetUploadQueue: React.FC<CreativeAssetUploadQueueProps> = ({
  items,
  labels,
  onCancel,
  onRetry,
  onDismiss,
}) => {
  if (items.length === 0) return null;

  return (
    <aside className={styles.uploadQueue} aria-label={labels.uploadQueue} data-asset-upload-queue>
      <header className={styles.uploadQueueHeader}>
        <strong>{labels.uploadQueue}</strong>
        <span>{items.length}</span>
      </header>
      <ul className={styles.uploadList}>
        {items.map((item) => {
          const percent = clampPercent(item.percent);
          return (
            <li key={item.id} className={styles.uploadItem} data-upload-status={item.status}>
              <span className={styles.uploadStatusIcon} aria-hidden='true'>
                {item.status === 'uploading' ? (
                  <Loading theme='outline' size={16} fill='currentColor' strokeWidth={3} />
                ) : item.status === 'completed' ? (
                  <CheckOne theme='outline' size={16} fill='currentColor' strokeWidth={3} />
                ) : (
                  <Error theme='outline' size={16} fill='currentColor' strokeWidth={3} />
                )}
              </span>
              <span className={styles.uploadBody}>
                <span className={styles.uploadMeta}>
                  <strong title={item.fileName}>{item.fileName}</strong>
                  <span>
                    {item.status === 'completed'
                      ? labels.uploadComplete
                      : item.status === 'error'
                        ? item.error
                        : `${Math.round(percent)}%`}
                  </span>
                </span>
                <span
                  className={styles.uploadProgress}
                  role='progressbar'
                  aria-valuemin={0}
                  aria-valuemax={100}
                  aria-valuenow={Math.round(percent)}
                >
                  <i style={{ width: `${percent}%` }} />
                </span>
              </span>
              {item.status === 'uploading' && onCancel ? (
                <button
                  type='button'
                  className={styles.iconButton}
                  aria-label={labels.cancelUpload}
                  title={labels.cancelUpload}
                  onClick={() => onCancel(item.id)}
                >
                  <Close theme='outline' size={15} fill='currentColor' strokeWidth={3} />
                </button>
              ) : item.status === 'error' && onRetry ? (
                <button
                  type='button'
                  className={styles.iconButton}
                  aria-label={labels.retryUpload}
                  title={labels.retryUpload}
                  onClick={() => onRetry(item.id)}
                >
                  <Refresh theme='outline' size={15} fill='currentColor' strokeWidth={3} />
                </button>
              ) : onDismiss ? (
                <button
                  type='button'
                  className={styles.iconButton}
                  aria-label={labels.dismissUpload}
                  title={labels.dismissUpload}
                  onClick={() => onDismiss(item.id)}
                >
                  <Close theme='outline' size={15} fill='currentColor' strokeWidth={3} />
                </button>
              ) : null}
            </li>
          );
        })}
      </ul>
    </aside>
  );
};

export default CreativeAssetUploadQueue;
