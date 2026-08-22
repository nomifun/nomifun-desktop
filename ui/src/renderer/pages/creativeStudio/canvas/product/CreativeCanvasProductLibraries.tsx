/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  FileText,
  Loading,
  Pic,
  Plus,
  Refresh,
  Search,
  VideoTwo,
  Voice,
} from '@icon-park/react';
import React, { useMemo } from 'react';

import {
  creativeAssetClient,
  type CreativeAsset,
  type CreativeAssetKind,
  type UseCreativeAssetsResult,
} from '../../assets';
import {
  createNomiPromptLibraryPort,
  PromptLibrarySidebar,
  type PromptLibrarySelection,
} from '../../prompts';
import styles from './CreativeCanvasProductLibraries.module.css';

export type CreativeCanvasAssetKindFilter = CreativeAssetKind | 'all';

export interface CreativeCanvasProductAssetLibraryProps {
  state: UseCreativeAssetsResult;
  search: string;
  kind: CreativeCanvasAssetKindFilter;
  selectedIds: ReadonlySet<string>;
  disabled?: boolean;
  onSearchChange(value: string): void;
  onKindChange(value: CreativeCanvasAssetKindFilter): void;
  onToggleAsset(assetId: string): void;
  onInsert(assets: readonly CreativeAsset[]): void;
}

const ASSET_KIND_LABELS: Record<CreativeCanvasAssetKindFilter, string> = {
  all: '全部类型',
  image: '图片',
  video: '视频',
  audio: '音频',
  text: '文本',
};

const iconProps = {
  theme: 'outline' as const,
  size: 18,
  fill: 'currentColor',
  strokeWidth: 2.5,
};

function assetIcon(kind: CreativeAssetKind): React.ReactNode {
  if (kind === 'image') return <Pic {...iconProps} />;
  if (kind === 'video') return <VideoTwo {...iconProps} />;
  if (kind === 'audio') return <Voice {...iconProps} />;
  return <FileText {...iconProps} />;
}

const CreativeAssetPreview: React.FC<{ asset: CreativeAsset }> = ({ asset }) => {
  const previewUrl = asset.thumbnailUrl ?? (asset.kind === 'image' ? asset.originalUrl : null);
  if (previewUrl) {
    return <img src={previewUrl} alt='' draggable={false} />;
  }
  return <span aria-hidden='true'>{assetIcon(asset.kind)}</span>;
};

/**
 * A read-only view of the authoritative NomiFun asset library. Mutations stay
 * in the asset product; this panel only selects real records and inserts them.
 */
export const CreativeCanvasProductAssetLibrary: React.FC<
  CreativeCanvasProductAssetLibraryProps
> = ({
  state,
  search,
  kind,
  selectedIds,
  disabled = false,
  onSearchChange,
  onKindChange,
  onToggleAsset,
  onInsert,
}) => {
  const selectedAssets = useMemo(
    () => state.assets.filter((asset) => selectedIds.has(asset.id)),
    [selectedIds, state.assets]
  );

  return (
    <section className={styles.assetPanel} aria-label='NomiFun 素材库' data-product-asset-library>
      <header className={styles.assetHeader}>
        <div>
          <strong>素材库</strong>
          <span>{state.total} 项真实素材</span>
        </div>
        <button
          type='button'
          aria-label='刷新素材库'
          disabled={disabled || state.loading}
          onClick={() => void state.reload()}
        >
          <Refresh {...iconProps} />
        </button>
      </header>

      <div className={styles.filters}>
        <label className={styles.searchField}>
          <Search {...iconProps} />
          <span className={styles.srOnly}>搜索素材</span>
          <input
            type='search'
            value={search}
            placeholder='搜索真实素材'
            disabled={disabled}
            onChange={(event) => onSearchChange(event.target.value)}
          />
        </label>
        <label className={styles.kindField}>
          <span className={styles.srOnly}>素材类型</span>
          <select
            value={kind}
            disabled={disabled}
            onChange={(event) =>
              onKindChange(event.target.value as CreativeCanvasAssetKindFilter)
            }
          >
            {(Object.keys(ASSET_KIND_LABELS) as CreativeCanvasAssetKindFilter[]).map(
              (value) => (
                <option key={value} value={value}>
                  {ASSET_KIND_LABELS[value]}
                </option>
              )
            )}
          </select>
        </label>
      </div>

      <div className={styles.assetBody}>
        {state.loading ? (
          <div className={styles.state} role='status' data-state='loading'>
            <Loading className={styles.spin} {...iconProps} />
            <span>正在读取素材库…</span>
          </div>
        ) : state.error ? (
          <div className={styles.state} role='alert' data-state='error'>
            <strong>素材库加载失败</strong>
            <span>{state.error.message}</span>
            <button type='button' onClick={() => void state.reload()}>
              重新加载
            </button>
          </div>
        ) : state.assets.length === 0 ? (
          <div className={styles.state} role='status' data-state='empty'>
            <strong>没有匹配的素材</strong>
            <span>这里只显示后端素材库返回的真实记录。</span>
          </div>
        ) : (
          <div className={styles.assetGrid} role='list'>
            {state.assets.map((asset) => {
              const selected = selectedIds.has(asset.id);
              return (
                <button
                  key={asset.id}
                  type='button'
                  className={styles.assetCard}
                  data-selected={selected || undefined}
                  aria-pressed={selected}
                  disabled={disabled}
                  onClick={() => onToggleAsset(asset.id)}
                  role='listitem'
                >
                  <span className={styles.assetPreview}>
                    <CreativeAssetPreview asset={asset} />
                  </span>
                  <span className={styles.assetCopy}>
                    <strong>{asset.title}</strong>
                    <span>{ASSET_KIND_LABELS[asset.kind]}</span>
                  </span>
                </button>
              );
            })}
          </div>
        )}
      </div>

      <footer className={styles.assetFooter}>
        {state.hasMore ? (
          <button
            type='button'
            className={styles.secondaryButton}
            disabled={disabled || state.loadingMore}
            onClick={() => void state.loadMore()}
          >
            {state.loadingMore ? <Loading className={styles.spin} {...iconProps} /> : null}
            加载更多
          </button>
        ) : (
          <span className={styles.endLabel}>已载入当前查询的全部素材</span>
        )}
        <button
          type='button'
          className={styles.insertButton}
          disabled={disabled || selectedAssets.length === 0}
          onClick={() => onInsert(selectedAssets)}
        >
          <Plus {...iconProps} />
          插入 {selectedAssets.length || ''}
        </button>
      </footer>
    </section>
  );
};
export interface CreativeCanvasProductPromptLibraryProps {
  locale: string;
  enabled?: boolean;
  selectedId?: string | null;
  onSelect?(id: string): void;
  onInsert(selection: PromptLibrarySelection): void;
}

/** Production prompt adapter: presets and text assets are loaded by the real port. */
export const CreativeCanvasProductPromptLibrary: React.FC<
  CreativeCanvasProductPromptLibraryProps
> = ({ locale, enabled = true, selectedId, onSelect, onInsert }) => {
  const port = useMemo(
    () => createNomiPromptLibraryPort({ locale, assets: creativeAssetClient }),
    [locale]
  );

  return (
    <div className={styles.promptPanel} data-product-prompt-library>
      <PromptLibrarySidebar
        port={port}
        enabled={enabled}
        title='提示词库'
        description='来自 NomiFun 预设与文本素材'
        selectedId={selectedId}
        onSelect={(item) => onSelect?.(item.id)}
        onInsert={onInsert}
      />
    </div>
  );
};
