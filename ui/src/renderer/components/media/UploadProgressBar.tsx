/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { abortUpload, useActiveUploads, useUploadState, type UploadSource } from '@/renderer/hooks/file/useUploadState';
import { CloseSmall } from '@icon-park/react';
import { useTranslation } from 'react-i18next';

/**
 * Thin progress bar shown while files are being uploaded. Renders nothing when
 * idle. Pass `source` to scope to a specific upload area (sendbox / workspace).
 *
 * In addition to the aggregated progress bar, each in-flight upload now gets a
 * row with its filename, percent, and an `x` button that aborts that single
 * upload via `AbortController.abort()`.
 */
const UploadProgressBar: React.FC<{ source?: UploadSource }> = ({ source }) => {
  const { isUploading, activeCount, overallPercent } = useUploadState(source);
  const activeUploads = useActiveUploads(source);
  const { t } = useTranslation();

  if (!isUploading) return null;

  // `color-` 是 UnoCSS 给 `text-` 的别名，所以 `color-text-3` 查的是一个叫「text-3」的
  // 颜色，产出 0 条 CSS；本仓库的 Arco 文字色规则就叫 text-1..text-4。
  // `color-` aliases `text-`, so `color-text-3` looked up a colour named "text-3".
  return (
    <div className='px-12px py-4px text-12px text-3'>
      <div className='flex justify-between mb-2px'>
        <span>
          {t('common.fileAttach.uploading', {
            count: activeCount,
            defaultValue: 'Uploading {{count}} file(s)...',
          })}
        </span>
        <span>{overallPercent}%</span>
      </div>
      <div className='h-3px rd-2px bg-fill-3 overflow-hidden'>
        <div
          className='h-full rd-2px bg-primary-6 transition-width duration-200 ease'
          style={{ width: `${overallPercent}%` }}
        />
      </div>
      {activeUploads.length > 0 && (
        <ul className='mt-6px flex flex-col gap-4px list-none p-0 m-0'>
          {activeUploads.map((upload) => (
            <li key={upload.id} className='flex items-center gap-8px py-2px' data-testid='upload-progress-item'>
              <span className='flex-1 min-w-0 truncate' title={upload.name}>
                {upload.name}
              </span>
              <span className='flex-shrink-0 tabular-nums'>{upload.percent}%</span>
              <button
                type='button'
                aria-label={t('common.fileAttach.cancelUpload', { defaultValue: 'Cancel upload' })}
                title={t('common.fileAttach.cancelUpload', { defaultValue: 'Cancel upload' })}
                className='flex-shrink-0 inline-flex items-center justify-center w-16px h-16px rd-full b-none bg-transparent cursor-pointer text-3 hover:text-1 hover:bg-fill-3 p-0'
                onClick={() => abortUpload(upload.id)}
                data-testid='upload-cancel-btn'
              >
                <CloseSmall theme='outline' size='12' strokeWidth={3} />
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
};

export default UploadProgressBar;
