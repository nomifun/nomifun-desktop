/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Button, Empty, Input, Modal, Spin } from '@arco-design/web-react';
import { Plus } from '@icon-park/react';
import React, { useEffect, useMemo, useRef, useState } from 'react';

import type { CreativeAsset, CreativeAssetKind } from '../types';
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

const KIND_LABELS: Record<CreativeAssetKind, string> = {
  image: '图片',
  video: '视频',
  audio: '音频',
  text: '文本',
};

function selectionHint(count: number, limit: number | undefined): string {
  if (limit === 1) return count > 0 ? '已选择 1 项' : '请选择 1 项素材';
  if (limit !== undefined) return `已选择 ${count}/${limit} 项`;
  return `已选择 ${count} 项`;
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
  title = '选择真实素材',
  selectionLimit,
  uploading = false,
  onToggle,
  onLoadMore,
  onRetry,
  onUploadFiles,
  onCancel,
  onConfirm,
}) => {
  const [scope, setScope] = useState<'mine' | 'library'>('mine');
  const [kind, setKind] = useState<'all' | CreativeAssetKind>('all');
  const [search, setSearch] = useState('');
  const uploadRef = useRef<HTMLInputElement>(null);
  const compatible = useMemo(() => {
    const query = search.trim().toLocaleLowerCase();
    return assets.filter((asset) => {
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
  const acceptedLabel = acceptedKinds.map((kind) => KIND_LABELS[kind]).join('、');
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
      title={title}
      footer={null}
      autoFocus={false}
      focusLock
      unmountOnExit
      getPopupContainer={() =>
        document.getElementById('creative-studio-portal-root') ?? document.body
      }
      onCancel={onCancel}
    >
      <div className={styles.scopeTabs} role='tablist' aria-label='素材范围'>
        <button
          type='button'
          role='tab'
          aria-selected={scope === 'mine'}
          data-active={scope === 'mine'}
          onClick={() => setScope('mine')}
        >
          我的素材
        </button>
        <button
          type='button'
          role='tab'
          aria-selected={scope === 'library'}
          data-active={scope === 'library'}
          onClick={() => setScope('library')}
        >
          素材库
        </button>
      </div>

      <Input.Search
        value={search}
        className={styles.search}
        allowClear
        placeholder='搜索素材'
        aria-label='搜索素材'
        onChange={setSearch}
      />

      <div className={styles.filterRow}>
        <div className={styles.kindFilters} role='group' aria-label='素材类型'>
          {(['all', ...acceptedKinds] as const).map((value) => (
            <button
              key={value}
              type='button'
              data-active={kind === value}
              onClick={() => setKind(value)}
            >
              {value === 'all' ? '全部' : KIND_LABELS[value]}
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
              新增素材
            </Button>
          </>
        ) : null}
      </div>

      <div className={styles.summary}>
        <span>可选择{acceptedLabel || '兼容素材'}</span>
        <strong>{selectionHint(selectedIds.length, selectionLimit)}</strong>
      </div>

      {loading && compatible.length === 0 ? (
        <div className={styles.empty} role='status'>
          <Spin />
          <span>正在载入素材…</span>
        </div>
      ) : error && compatible.length === 0 ? (
        <div className={styles.empty} role='alert'>
          <strong>素材加载失败</strong>
          <span>{error.message}</span>
          {onRetry ? <Button onClick={onRetry}>重试</Button> : null}
        </div>
      ) : compatible.length === 0 ? (
        <div className={styles.empty} role='status'>
          <Empty description={search || kind !== 'all' ? '没有匹配的素材' : '没有素材'} />
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
                aria-label={`${asset.title}，${selected ? '已选择' : '未选择'}`}
                disabled={limitReached}
                onClick={() => onToggle(asset)}
              >
                <span className={styles.media}>
                  <CreativeAssetMedia asset={asset} compact unavailableLabel='素材文件不可用' />
                </span>
                <span className={styles.identity}>
                  <strong title={asset.title}>{asset.title}</strong>
                  <small>{KIND_LABELS[asset.kind]} · {selected ? '已选择' : '点击选择'}</small>
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
              加载更多
            </Button>
          ) : null}
        </div>
        <div className={styles.actions}>
          {onConfirm ? <Button onClick={onCancel}>取消</Button> : null}
          <Button type='primary' onClick={onConfirm ?? onCancel}>
            完成
          </Button>
        </div>
      </footer>
    </Modal>
  );
};

export default CreativeAssetPickerModal;
