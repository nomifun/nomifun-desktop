/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useMemo } from 'react';
import { useTranslation } from 'react-i18next';

import CreativePromptPicker from '../../components/CreativePromptPicker';
import {
  CreativeAssetPickerContent,
  isCreativeAssetDeleted,
  type CreativeAsset,
  type CreativeAssetKind,
  type UseCreativeAssetsResult,
} from '../../assets';
import {
  toPromptLibrarySelection,
  type PromptLibraryPort,
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
  onCancel?(): void;
}

const ALL_ASSET_KINDS: readonly CreativeAssetKind[] = [
  'image',
  'video',
  'audio',
  'text',
];

/**
 * The Canvas entry point for the authoritative asset picker. It shares the
 * conversation picker content while keeping Canvas insertion as its only
 * product-specific action.
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
  onCancel,
}) => {
  const { t } = useTranslation();
  const selectedAssets = useMemo(
    () => state.assets.filter((asset) => !isCreativeAssetDeleted(asset) && selectedIds.has(asset.id)),
    [selectedIds, state.assets]
  );

  return (
    <section
      aria-label={t('creativeStudio.canvas.assets.libraryLabel', {
        defaultValue: 'NomiFun 资产库',
      })}
      data-product-asset-library
    >
      <CreativeAssetPickerContent
        open
        assets={state.assets}
        acceptedKinds={ALL_ASSET_KINDS}
        selectedIds={[...selectedIds]}
        loading={state.loading}
        loadingMore={state.loadingMore}
        hasMore={state.hasMore}
        error={state.error ?? state.mutationError}
        disabled={disabled}
        uploading={state.mutating}
        search={search}
        kind={kind}
        uploadAccept='image/*,video/*'
        onSearchChange={onSearchChange}
        onKindChange={onKindChange}
        onToggle={(asset) => onToggleAsset(asset.id)}
        onLoadMore={() => void state.loadMore()}
        onRetry={() => void state.reload()}
        onUploadFiles={(files) => {
          void Promise.all(
            files.map((file) => state.upload(file, {
              title: file.name,
              tags: ['canvas-import'],
              inLibrary: true,
            }))
          ).catch(() => undefined);
        }}
        onCancel={onCancel}
        onConfirm={() => {
          if (selectedAssets.length > 0) onInsert(selectedAssets);
          onCancel?.();
        }}
      />
    </section>
  );
};
export interface CreativeCanvasProductPromptLibraryProps {
  locale: string;
  enabled?: boolean;
  selectedId?: string | null;
  port?: PromptLibraryPort;
  onSelect?(id: string): void;
  onCopy(selection: PromptLibrarySelection): void;
}

/** Production prompt adapter: presets and text assets are loaded by the real port. */
export const CreativeCanvasProductPromptLibrary: React.FC<
  CreativeCanvasProductPromptLibraryProps
> = ({ locale, enabled = true, selectedId, port, onSelect, onCopy }) => (
  <div className={styles.promptPanel} data-product-prompt-library>
    <CreativePromptPicker
      port={port}
      locale={locale}
      enabled={enabled}
      selectedId={selectedId}
      applyLabel='复制提示词'
      onSelect={(item) => {
        onSelect?.(item.id);
        onCopy(toPromptLibrarySelection(item));
      }}
    />
  </div>
);
