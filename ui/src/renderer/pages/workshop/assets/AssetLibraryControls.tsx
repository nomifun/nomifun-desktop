/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Shared presentational controls for the two asset-library surfaces: the
 * standalone page (`pages/assets`) and the in-canvas drawer (`AssetsPanel`).
 * The `variant` prop carries the (intentional) visual differences between the
 * roomy page layout and the compact drawer strip.
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { Close } from '@icon-park/react';

import type { useAssetLibrary, AssetKindFilter } from './useAssetLibrary';

export type AssetControlVariant = 'page' | 'drawer';

// ─── Segmented kind filter ────────────────────────────────────────────────────

export const KIND_SEGMENTS: AssetKindFilter[] = ['all', 'image', 'video', 'text'];

export const SegmentedKindFilter: React.FC<{
  value: AssetKindFilter;
  onChange: (v: AssetKindFilter) => void;
  labelOf: (k: AssetKindFilter) => string;
  variant?: AssetControlVariant;
}> = ({ value, onChange, labelOf, variant = 'page' }) => (
  <div className='flex items-center gap-2px rounded-9px border border-solid border-[var(--color-border-2)] bg-[var(--color-fill-1)] p-2px'>
    {KIND_SEGMENTS.map((k) => {
      const active = value === k;
      return (
        <div
          key={k}
          role='button'
          tabIndex={0}
          onClick={() => onChange(k)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' || e.key === ' ') {
              e.preventDefault();
              onChange(k);
            }
          }}
          className={[
            variant === 'drawer' ? 'flex-1 px-2px' : 'px-12px',
            'select-none rounded-7px py-4px text-center text-12px font-500 cursor-pointer transition-all duration-120',
            active
              ? 'bg-[var(--color-bg-2)] text-[var(--color-text-1)] shadow-[0_1px_4px_rgba(0,0,0,0.1)]'
              : 'text-[var(--color-text-3)] hover:text-[var(--color-text-1)]',
          ].join(' ')}
        >
          {labelOf(k)}
        </div>
      );
    })}
  </div>
);

// ─── Upload tray ──────────────────────────────────────────────────────────────

const TRAY_CONTAINER: Record<AssetControlVariant, string> = {
  // Standalone rounded card in the page flow.
  page: 'flex flex-col gap-8px rounded-12px border border-solid border-[var(--color-border-2)] bg-[var(--color-fill-1)] px-16px py-12px',
  // Edge-to-edge strip under the drawer toolbar.
  drawer: 'flex flex-col gap-6px border-b border-b-solid border-[var(--color-border-2)] bg-[var(--color-fill-1)] px-14px py-10px',
};

export const UploadTray: React.FC<{
  uploads: ReturnType<typeof useAssetLibrary>['uploads'];
  onCancel: (localId: string) => void;
  onClearDone: () => void;
  t: ReturnType<typeof useTranslation>['t'];
  variant?: AssetControlVariant;
}> = ({ uploads, onCancel, onClearDone, t, variant = 'page' }) => {
  if (uploads.length === 0) return null;
  const hasFinished = uploads.some((u) => u.status !== 'uploading');
  return (
    <div className={TRAY_CONTAINER[variant]}>
      <div className='flex items-center justify-between'>
        <span className='text-11px font-600 uppercase tracking-wide text-[var(--color-text-4)]'>
          {t('workshopAssets.upload.queue', { defaultValue: '上传队列' })}
        </span>
        {hasFinished && (
          <div
            role='button'
            tabIndex={0}
            onClick={onClearDone}
            onKeyDown={(e) => {
              if (e.key === 'Enter' || e.key === ' ') {
                e.preventDefault();
                onClearDone();
              }
            }}
            className='text-11px text-[var(--color-text-3)] cursor-pointer hover:text-[var(--color-text-1)]'
          >
            {t('workshopAssets.upload.clearDone', { defaultValue: '清除已完成' })}
          </div>
        )}
      </div>
      {uploads.map((u) => (
        <div key={u.localId} className={variant === 'drawer' ? 'flex items-center gap-8px' : 'flex items-center gap-10px'}>
          <div className='min-w-0 flex-1'>
            <div className='flex items-center justify-between gap-8px'>
              <span className='truncate text-12px text-[var(--color-text-2)]'>{u.fileName}</span>
              <span
                className={[
                  'shrink-0 text-11px font-600',
                  u.status === 'error' ? 'text-danger-6' : 'text-[var(--color-text-3)]',
                ].join(' ')}
              >
                {u.status === 'error'
                  ? t(`workshopAssets.upload.${u.error ?? 'failed'}`, { defaultValue: '上传失败' })
                  : `${u.percent}%`}
              </span>
            </div>
            <div className='mt-4px h-4px w-full overflow-hidden rounded-full bg-[var(--color-fill-3)]'>
              <div
                className={[
                  'h-full rounded-full transition-all duration-200',
                  u.status === 'error' ? 'bg-danger-6' : 'bg-primary-6',
                ].join(' ')}
                style={{ width: `${u.status === 'error' ? 100 : u.percent}%` }}
              />
            </div>
          </div>
          <div
            role='button'
            tabIndex={0}
            title={t('workshopAssets.upload.cancel', { defaultValue: '取消上传' })}
            onClick={() => onCancel(u.localId)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' || e.key === ' ') {
                e.preventDefault();
                onCancel(u.localId);
              }
            }}
            className='grid h-22px w-22px shrink-0 place-items-center rounded-6px text-[var(--color-text-3)] cursor-pointer hover:bg-[var(--color-fill-2)] hover:text-[var(--color-text-1)]'
          >
            <Close theme='outline' size={13} strokeWidth={3} />
          </div>
        </div>
      ))}
    </div>
  );
};
