/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Button, Empty, Input, Modal, Spin } from '@arco-design/web-react';
import { Plus } from '@icon-park/react';
import type { TFunction } from 'i18next';
import React, { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { isCreativeAssetDeleted, type CreativeAsset, type CreativeAssetKind } from '../types';
import CreativeAssetMedia from './CreativeAssetMedia';
import styles from './CreativeAssetPickerModal.module.css';

export interface CreativeAssetPickerModalProps {
  open: boolean;
  assets: readonly CreativeAsset[];
  acceptedKinds: readonly CreativeAssetKind[];
  selectedIds: readonly string[];
  loading: boolean;
  loadingMore?: boolean;
  hasMore: boolean;
  error?: Error | null;
  title?: string;
  selectionLimit?: number;
  uploading?: boolean;
  onToggle(asset: CreativeAsset): void;
  onLoadMore(): void;
  onRetry?(): void;
  onUploadFiles?(files: readonly File[]): void;
  onCancel(): void;
  onConfirm?(): void;
}

function selectionHint(t: TFunction, count: number, limit: number | undefined): string {
  if (limit === 1) {
    return count > 0
      ? t('creativeStudio.assets.picker.oneSelected', { defaultValue: '已选择 1 项' })
      : t('creativeStudio.assets.picker.oneRequired', { defaultValue: '请选择 1 项素材' });
  }
  if (limit !== undefined) {
    return t('creativeStudio.assets.picker.limitedSelection', {
      defaultValue: '已选择 {{selected}}/{{limit}} 项',
      selected: count,
      limit,
    });
  }
  return t('creativeStudio.assets.picker.selectedCount', {
    defaultValue: '已选择 {{itemCount}} 项',
    itemCount: count,
  });
}

const CreativeAssetPickerModal: React.FC<CreativeAssetPickerModalProps> = ({
  open,
  assets,
  acceptedKinds,
  selectedIds,
  loading,
  loadingMore = false,
  hasMore,
  error = null,
  title,
  selectionLimit,
  uploading = false,
  onToggle,
  onLoadMore,
  onRetry,
  onUploadFiles,
  onCancel,
  onConfirm,
}) => {
  const { t, i18n } = useTranslation();
  const [scope, setScope] = useState<'mine' | 'library'>('mine');
  const [kind, setKind] = useState<'all' | CreativeAssetKind>('all');
  const [search, setSearch] = useState('');
  const uploadRef = useRef<HTMLInputElement>(null);
  const kindLabels = useMemo<Record<CreativeAssetKind, string>>(
    () => ({
      image: t('creativeStudio.assets.kind.image', { defaultValue: '图片' }),
      video: t('creativeStudio.assets.kind.video', { defaultValue: '视频' }),
      audio: t('creativeStudio.assets.kind.audio', { defaultValue: '音频' }),
      text: t('creativeStudio.assets.kind.text', { defaultValue: '文本' }),
    }),
    [t]
  );
  const compatible = useMemo(() => {
    const query = search.trim().toLocaleLowerCase();
    return assets.filter((asset) => {
      if (isCreativeAssetDeleted(asset)) return false;
      if (!acceptedKinds.includes(asset.kind)) return false;
      if (scope === 'library' && !asset.inLibrary) return false;
      if (kind !== 'all' && asset.kind !== kind) return false;
      if (!query) return true;
      return [asset.title, asset.collection ?? '', ...asset.tags]
        .join('\n')
        .toLocaleLowerCase()
        .includes(query);
    });
  }, [acceptedKinds, assets, kind, scope, search]);
  const acceptedLabel = useMemo(() => {
    const labels = acceptedKinds.map((acceptedKind) => kindLabels[acceptedKind]);
    try {
      return new Intl.ListFormat(i18n.resolvedLanguage ?? i18n.language, {
        style: 'short',
        type: 'conjunction',
      }).format(labels);
    } catch {
      return labels.join('、');
    }
  }, [acceptedKinds, i18n.language, i18n.resolvedLanguage, kindLabels]);
  const accept = acceptedKinds
    .map((acceptedKind) => {
      if (acceptedKind === 'image') return 'image/*';
      if (acceptedKind === 'video') return 'video/*';
      if (acceptedKind === 'audio') return 'audio/*';
      return 'text/plain';
    })
    .join(',');

  useEffect(() => {
    if (!open) return;
    setScope('mine');
    setKind('all');
    setSearch('');
  }, [open]);

  return (
    <Modal
      visible={open}
      alignCenter={false}
      className={styles.modal}
      title={title ?? t('creativeStudio.assets.picker.title', { defaultValue: '选择真实素材' })}
      footer={null}
      autoFocus={false}
      focusLock
      unmountOnExit
      getPopupContainer={() =>
        document.getElementById('creative-studio-portal-root') ?? document.body
      }
      onCancel={onCancel}
    >
      <div
        className={styles.scopeTabs}
        role='tablist'
        aria-label={t('creativeStudio.assets.picker.scopeLabel', { defaultValue: '素材范围' })}
      >
        <button
          type='button'
          role='tab'
          aria-selected={scope === 'mine'}
          data-active={scope === 'mine'}
          onClick={() => setScope('mine')}
        >
          {t('creativeStudio.assets.picker.myAssets', { defaultValue: '我的素材' })}
        </button>
        <button
          type='button'
          role='tab'
          aria-selected={scope === 'library'}
          data-active={scope === 'library'}
          onClick={() => setScope('library')}
        >
          {t('creativeStudio.assets.picker.library', { defaultValue: '素材库' })}
        </button>
      </div>

      <Input.Search
        value={search}
        className={styles.search}
        allowClear
        placeholder={t('creativeStudio.assets.picker.searchPlaceholder', { defaultValue: '搜索素材' })}
        aria-label={t('creativeStudio.assets.picker.searchLabel', { defaultValue: '搜索素材' })}
        onChange={setSearch}
      />

      <div className={styles.filterRow}>
        <div
          className={styles.kindFilters}
          role='group'
          aria-label={t('creativeStudio.assets.picker.kindLabel', { defaultValue: '素材类型' })}
        >
          {(['all', ...acceptedKinds] as const).map((value) => (
            <button
              key={value}
              type='button'
              data-active={kind === value}
              onClick={() => setKind(value)}
            >
              {value === 'all'
                ? t('creativeStudio.assets.kind.all', { defaultValue: '全部' })
                : kindLabels[value]}
            </button>
          ))}
        </div>
        {onUploadFiles ? (
          <>
            <input
              ref={uploadRef}
              hidden
              type='file'
              multiple={selectionLimit !== 1}
              accept={accept}
              onChange={(event) => {
                const files = [...(event.currentTarget.files ?? [])];
                event.currentTarget.value = '';
                if (files.length > 0) onUploadFiles(files);
              }}
            />
            <Button
              size='small'
              loading={uploading}
              disabled={uploading}
              icon={<Plus theme='outline' size={13} fill='currentColor' />}
              onClick={() => uploadRef.current?.click()}
            >
              {t('creativeStudio.assets.picker.addAsset', { defaultValue: '新增素材' })}
            </Button>
          </>
        ) : null}
      </div>

      <div className={styles.summary}>
        <span>
          {t('creativeStudio.assets.picker.acceptedKinds', {
            defaultValue: '可选择{{kinds}}',
            kinds:
              acceptedLabel ||
              t('creativeStudio.assets.picker.compatibleAssets', { defaultValue: '兼容素材' }),
          })}
        </span>
        <strong>{selectionHint(t, selectedIds.length, selectionLimit)}</strong>
      </div>

      {loading && compatible.length === 0 ? (
        <div className={styles.empty} role='status'>
          <Spin />
          <span>{t('creativeStudio.assets.picker.loading', { defaultValue: '正在载入素材…' })}</span>
        </div>
      ) : error && compatible.length === 0 ? (
        <div className={styles.empty} role='alert'>
          <strong>{t('creativeStudio.assets.picker.loadFailed', { defaultValue: '素材加载失败' })}</strong>
          <span>{error.message}</span>
          {onRetry ? (
            <Button onClick={onRetry}>
              {t('creativeStudio.assets.picker.retry', { defaultValue: '重试' })}
            </Button>
          ) : null}
        </div>
      ) : compatible.length === 0 ? (
        <div className={styles.empty} role='status'>
          <Empty
            description={
              search || kind !== 'all'
                ? t('creativeStudio.assets.picker.noMatches', { defaultValue: '没有匹配的素材' })
                : t('creativeStudio.assets.picker.empty', { defaultValue: '没有素材' })
            }
          />
        </div>
      ) : (
        <div className={styles.grid} role='listbox' aria-multiselectable={selectionLimit !== 1}>
          {compatible.map((asset) => {
            const selected = selectedIds.includes(asset.id);
            const limitReached =
              !selected && selectionLimit !== undefined && selectedIds.length >= selectionLimit;
            return (
              <button
                key={asset.id}
                type='button'
                className={styles.item}
                data-selected={selected}
                role='option'
                aria-selected={selected}
                aria-label={t('creativeStudio.assets.picker.assetSelectionLabel', {
                  defaultValue: '{{title}}，{{state}}',
                  title: asset.title,
                  state: selected
                    ? t('creativeStudio.assets.picker.selected', { defaultValue: '已选择' })
                    : t('creativeStudio.assets.picker.notSelected', { defaultValue: '未选择' }),
                })}
                disabled={limitReached}
                onClick={() => onToggle(asset)}
              >
                <span className={styles.media}>
                  <CreativeAssetMedia
                    asset={asset}
                    compact
                    unavailableLabel={t('creativeStudio.assets.picker.mediaUnavailable', {
                      defaultValue: '素材文件不可用',
                    })}
                  />
                </span>
                <span className={styles.identity}>
                  <strong title={asset.title}>{asset.title}</strong>
                  <small>
                    {kindLabels[asset.kind]} ·{' '}
                    {selected
                      ? t('creativeStudio.assets.picker.selected', { defaultValue: '已选择' })
                      : t('creativeStudio.assets.picker.clickToSelect', {
                          defaultValue: '点击选择',
                        })}
                  </small>
                </span>
              </button>
            );
          })}
        </div>
      )}

      <footer className={styles.footer}>
        <div>
          {hasMore ? (
            <Button loading={loadingMore} disabled={loadingMore} onClick={onLoadMore}>
              {t('creativeStudio.assets.picker.loadMore', { defaultValue: '加载更多' })}
            </Button>
          ) : null}
        </div>
        <div className={styles.actions}>
          {onConfirm ? (
            <Button onClick={onCancel}>
              {t('creativeStudio.assets.picker.cancel', { defaultValue: '取消' })}
            </Button>
          ) : null}
          <Button type='primary' onClick={onConfirm ?? onCancel}>
            {t('creativeStudio.assets.picker.done', { defaultValue: '完成' })}
          </Button>
        </div>
      </footer>
    </Modal>
  );
};

export default CreativeAssetPickerModal;
