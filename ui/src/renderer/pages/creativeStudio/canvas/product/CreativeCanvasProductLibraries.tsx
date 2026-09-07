/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  Loading,
  Plus,
  Refresh,
  Search,
} from '@icon-park/react';
import React, { useMemo } from 'react';
import { useTranslation } from 'react-i18next';

import {
  creativeAssetClient,
  isCreativeAssetDeleted,
  type CreativeAsset,
  type CreativeAssetKind,
  type UseCreativeAssetsResult,
} from '../../assets';
import CreativeAssetMedia from '../../assets/components/CreativeAssetMedia';
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

const ASSET_KIND_LABEL_KEYS: Record<
  CreativeCanvasAssetKindFilter,
  string
> = {
  all: 'creativeStudio.canvas.assets.kind.all',
  image: 'creativeStudio.canvas.assets.kind.image',
  video: 'creativeStudio.canvas.assets.kind.video',
  audio: 'creativeStudio.canvas.assets.kind.audio',
  text: 'creativeStudio.canvas.assets.kind.text',
};

const assetKindFallbacks: Record<CreativeCanvasAssetKindFilter, string> = {
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
  const { t } = useTranslation();
  const assetKindLabel = (value: CreativeCanvasAssetKindFilter): string =>
    t(ASSET_KIND_LABEL_KEYS[value], { defaultValue: assetKindFallbacks[value] });
  const selectedAssets = useMemo(
    () => state.assets.filter((asset) => !isCreativeAssetDeleted(asset) && selectedIds.has(asset.id)),
    [selectedIds, state.assets]
  );

  return (
    <section
      className={styles.assetPanel}
      aria-label={t('creativeStudio.canvas.assets.libraryLabel', {
        defaultValue: 'NomiFun 素材库',
      })}
      data-product-asset-library
    >
      <header className={styles.assetHeader}>
        <div>
          <strong>
            {t('creativeStudio.canvas.assets.libraryTitle', {
              defaultValue: '素材库',
            })}
          </strong>
          <span>
            {t('creativeStudio.canvas.assets.totalCount', {
              count: state.total,
              defaultValue: `${state.total} 项真实素材`,
            })}
          </span>
        </div>
        <button
          type='button'
          aria-label={t('creativeStudio.canvas.assets.refresh', {
            defaultValue: '刷新素材库',
          })}
          disabled={disabled || state.loading}
          onClick={() => void state.reload()}
        >
          <Refresh {...iconProps} />
        </button>
      </header>

      <div className={styles.filters}>
        <label className={styles.searchField}>
          <Search {...iconProps} />
          <span className={styles.srOnly}>
            {t('creativeStudio.canvas.assets.searchLabel', {
              defaultValue: '搜索素材',
            })}
          </span>
          <input
            type='search'
            value={search}
            placeholder={t('creativeStudio.canvas.assets.searchPlaceholder', {
              defaultValue: '搜索真实素材',
            })}
            disabled={disabled}
            onChange={(event) => onSearchChange(event.target.value)}
          />
        </label>
        <label className={styles.kindField}>
          <span className={styles.srOnly}>
            {t('creativeStudio.canvas.assets.kindLabel', {
              defaultValue: '素材类型',
            })}
          </span>
          <select
            value={kind}
            disabled={disabled}
            onChange={(event) =>
              onKindChange(event.target.value as CreativeCanvasAssetKindFilter)
            }
          >
            {(
              Object.keys(ASSET_KIND_LABEL_KEYS) as CreativeCanvasAssetKindFilter[]
            ).map((value) => (
                <option key={value} value={value}>
                  {assetKindLabel(value)}
                </option>
              ))}
          </select>
        </label>
      </div>

      <div className={styles.assetBody}>
        {state.loading ? (
          <div className={styles.state} role='status' data-state='loading'>
            <Loading className={styles.spin} {...iconProps} />
            <span>
              {t('creativeStudio.canvas.assets.loading', {
                defaultValue: '正在读取素材库…',
              })}
            </span>
          </div>
        ) : state.error ? (
          <div className={styles.state} role='alert' data-state='error'>
            <strong>
              {t('creativeStudio.canvas.assets.loadFailed', {
                defaultValue: '素材库加载失败',
              })}
            </strong>
            <span>{state.error.message}</span>
            <button type='button' onClick={() => void state.reload()}>
              {t('creativeStudio.canvas.assets.reload', {
                defaultValue: '重新加载',
              })}
            </button>
          </div>
        ) : state.assets.length === 0 ? (
          <div className={styles.state} role='status' data-state='empty'>
            <strong>
              {t('creativeStudio.canvas.assets.noMatches', {
                defaultValue: '没有匹配的素材',
              })}
            </strong>
            <span>
              {t('creativeStudio.canvas.assets.realRecordsOnly', {
                defaultValue: '这里只显示后端素材库返回的真实记录。',
              })}
            </span>
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
                  disabled={disabled || isCreativeAssetDeleted(asset)}
                  onClick={() => onToggleAsset(asset.id)}
                  role='listitem'
                >
                  <div className={styles.assetPreview}>
                    <CreativeAssetMedia
                      asset={asset}
                      compact
                      unavailableLabel={t('creativeStudio.assets.library.mediaUnavailable', {
                        defaultValue: '素材暂时无法预览',
                      })}
                    />
                  </div>
                  <span className={styles.assetCopy}>
                    <strong>{asset.title}</strong>
                    <span>{assetKindLabel(asset.kind)}</span>
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
            {t('creativeStudio.canvas.assets.loadMore', {
              defaultValue: '加载更多',
            })}
          </button>
        ) : (
          <span className={styles.endLabel}>
            {t('creativeStudio.canvas.assets.allLoaded', {
              defaultValue: '已载入当前查询的全部素材',
            })}
          </span>
        )}
        <button
          type='button'
          className={styles.insertButton}
          disabled={disabled || selectedAssets.length === 0}
          onClick={() => onInsert(selectedAssets)}
        >
          <Plus {...iconProps} />
          {t('creativeStudio.canvas.assets.insert', {
            count: selectedAssets.length,
            defaultValue: '插入 {{count}}',
          })}
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
  onCopy(selection: PromptLibrarySelection): void;
}

/** Production prompt adapter: presets and text assets are loaded by the real port. */
export const CreativeCanvasProductPromptLibrary: React.FC<
  CreativeCanvasProductPromptLibraryProps
> = ({ locale, enabled = true, selectedId, onSelect, onCopy }) => {
  const { t } = useTranslation();
  const port = useMemo(
    () => createNomiPromptLibraryPort({ locale, assets: creativeAssetClient }),
    [locale]
  );

  return (
    <div className={styles.promptPanel} data-product-prompt-library>
      <PromptLibrarySidebar
        port={port}
        enabled={enabled}
        title={t('creativeStudio.canvas.promptLibrary', {
          defaultValue: '提示词库',
        })}
        description={t('creativeStudio.canvas.promptLibraryDescription', {
          defaultValue: '来自 NomiFun 预设与文本素材',
        })}
        selectedId={selectedId}
        onSelect={(item) => onSelect?.(item.id)}
        onCopy={onCopy}
      />
    </div>
  );
};
