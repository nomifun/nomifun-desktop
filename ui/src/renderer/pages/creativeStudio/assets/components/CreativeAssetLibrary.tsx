/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  AllApplication,
  Check,
  Close,
  Delete,
  Download,
  EditTwo,
  Error,
  GridNine,
  List,
  Loading,
  Pic,
  Plus,
  Search,
  Text,
  Upload,
  VideoTwo,
  Voice,
} from '@icon-park/react';
import classNames from 'classnames';
import React, { useMemo, useRef, useState } from 'react';

import type { CreativeAsset, CreativeAssetKind } from '../types';
import CreativeAssetMedia, { creativeAssetKindIcon } from './CreativeAssetMedia';
import CreativeAssetUploadQueue from './CreativeAssetUploadQueue';
import type {
  CreativeAssetAction,
  CreativeAssetBatchAction,
  CreativeAssetKindFilter,
  CreativeAssetLibraryLabels,
  CreativeAssetLibraryState,
  CreativeAssetScope,
  CreativeAssetUploadItem,
  CreativeAssetViewMode,
} from './types';
import { DEFAULT_CREATIVE_ASSET_LIBRARY_LABELS } from './types';
import styles from './CreativeAssetLibrary.module.css';

export interface CreativeAssetLibraryProps {
  state: CreativeAssetLibraryState;
  search: string;
  kind: CreativeAssetKindFilter;
  scope: CreativeAssetScope;
  view: CreativeAssetViewMode;
  selectedIds: ReadonlySet<string>;
  uploads?: readonly CreativeAssetUploadItem[];
  disabled?: boolean;
  className?: string;
  locale?: string;
  uploadAccept?: string;
  labels?: Partial<CreativeAssetLibraryLabels>;
  onSearchChange: (value: string) => void;
  onKindChange: (kind: CreativeAssetKindFilter) => void;
  onScopeChange: (scope: CreativeAssetScope) => void;
  onViewChange: (view: CreativeAssetViewMode) => void;
  onSelectionChange: (selectedIds: ReadonlySet<string>) => void;
  onUploadFiles?: (files: readonly File[]) => void;
  onCreateText?: () => void;
  onOpenAsset?: CreativeAssetAction;
  onEditAsset?: CreativeAssetAction;
  onDownloadAsset?: CreativeAssetAction;
  onRemoveAsset?: CreativeAssetAction;
  onSetSelectedLibrary?: (assets: readonly CreativeAsset[], inLibrary: boolean) => void;
  onInsertSelected?: CreativeAssetBatchAction;
  onDownloadSelected?: CreativeAssetBatchAction;
  onRemoveSelected?: CreativeAssetBatchAction;
  onCancelUpload?: (uploadId: string) => void;
  onRetryUpload?: (uploadId: string) => void;
  onDismissUpload?: (uploadId: string) => void;
}

const KIND_FILTERS: Array<{ value: CreativeAssetKindFilter; icon: React.ReactNode }> = [
  { value: 'all', icon: <AllApplication theme='outline' size={15} fill='currentColor' strokeWidth={3} /> },
  { value: 'image', icon: <Pic theme='outline' size={15} fill='currentColor' strokeWidth={3} /> },
  { value: 'video', icon: <VideoTwo theme='outline' size={15} fill='currentColor' strokeWidth={3} /> },
  { value: 'audio', icon: <Voice theme='outline' size={15} fill='currentColor' strokeWidth={3} /> },
  { value: 'text', icon: <Text theme='outline' size={15} fill='currentColor' strokeWidth={3} /> },
];

const formatBytes = (bytes: number | null): string => {
  if (bytes == null || bytes < 0 || !Number.isFinite(bytes)) return '—';
  if (bytes < 1024) return `${bytes} B`;
  const units = ['KB', 'MB', 'GB', 'TB'];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value >= 10 ? value.toFixed(0) : value.toFixed(1)} ${units[unit]}`;
};

const assetDetails = (asset: CreativeAsset): string => {
  const dimensions = asset.width && asset.height ? `${asset.width} × ${asset.height}` : null;
  return [dimensions, formatBytes(asset.bytes), asset.mimeType].filter(Boolean).join(' · ');
};

const kindLabel = (kind: CreativeAssetKind, labels: CreativeAssetLibraryLabels) => labels[kind];

const formatUpdatedAt = (timestamp: number, locale?: string): { label: string; dateTime?: string } => {
  const date = new Date(timestamp);
  if (!Number.isFinite(timestamp) || Number.isNaN(date.getTime())) return { label: '—' };
  try {
    return {
      label: new Intl.DateTimeFormat(locale, { dateStyle: 'medium' }).format(date),
      dateTime: date.toISOString(),
    };
  } catch {
    return { label: date.toISOString().slice(0, 10), dateTime: date.toISOString() };
  }
};

interface AssetItemProps {
  asset: CreativeAsset;
  view: CreativeAssetViewMode;
  selected: boolean;
  disabled: boolean;
  locale?: string;
  labels: CreativeAssetLibraryLabels;
  onToggle: () => void;
  onOpen?: CreativeAssetAction;
  onEdit?: CreativeAssetAction;
  onDownload?: CreativeAssetAction;
  onRemove?: CreativeAssetAction;
}

const AssetItem: React.FC<AssetItemProps> = ({
  asset,
  view,
  selected,
  disabled,
  locale,
  labels,
  onToggle,
  onOpen,
  onEdit,
  onDownload,
  onRemove,
}) => {
  const updatedAt = formatUpdatedAt(asset.updatedAt, locale);
  return (
    <article
      className={classNames(styles.assetItem, view === 'list' && styles.assetRow)}
      data-asset-id={asset.id}
      data-asset-kind={asset.kind}
      data-selected={selected || undefined}
    >
      <label className={styles.assetSelect} title={labels.select}>
        <input type='checkbox' checked={selected} disabled={disabled} onChange={onToggle} />
        <span aria-hidden='true'>
          <Check theme='outline' size={13} fill='currentColor' strokeWidth={4} />
        </span>
        <span className={styles.srOnly}>{labels.select}</span>
      </label>

      <button
        type='button'
        className={styles.assetPreviewButton}
        disabled={!onOpen || disabled}
        aria-label={`${labels.open}: ${asset.title}`}
        onClick={() => onOpen?.(asset)}
      >
        <CreativeAssetMedia asset={asset} unavailableLabel={labels.mediaUnavailable} compact={view === 'list'} />
      </button>

      <div className={styles.assetContent}>
        <div className={styles.assetHeading}>
          <div className={styles.assetTitleBlock}>
            <strong title={asset.title}>{asset.title}</strong>
            <span>{asset.collection || labels.noCollection}</span>
          </div>
          <span className={styles.kindBadge} data-kind={asset.kind}>
            <span aria-hidden='true'>{creativeAssetKindIcon(asset.kind, 13)}</span>
            {kindLabel(asset.kind, labels)}
          </span>
        </div>

        <p className={styles.assetDetails}>{assetDetails(asset)}</p>
        <div className={styles.assetTags} aria-label={asset.tags.length ? asset.tags.join(', ') : labels.noTags}>
          {asset.tags.length ? (
            asset.tags.slice(0, view === 'list' ? 4 : 3).map((tag) => <span key={tag}>{tag}</span>)
          ) : (
            <span>{labels.noTags}</span>
          )}
        </div>
        {view === 'list' ? <time dateTime={updatedAt.dateTime}>{updatedAt.label}</time> : null}
      </div>

      <div className={styles.assetActions}>
        {onEdit ? (
          <button type='button' disabled={disabled} title={labels.edit} aria-label={labels.edit} onClick={() => onEdit(asset)}>
            <EditTwo theme='outline' size={15} fill='currentColor' strokeWidth={3} />
          </button>
        ) : null}
        {onDownload && asset.kind !== 'text' ? (
          <button
            type='button'
            disabled={disabled}
            title={labels.download}
            aria-label={labels.download}
            onClick={() => onDownload(asset)}
          >
            <Download theme='outline' size={15} fill='currentColor' strokeWidth={3} />
          </button>
        ) : null}
        {onRemove ? (
          <button
            type='button'
            disabled={disabled}
            title={labels.remove}
            aria-label={labels.remove}
            data-danger
            onClick={() => onRemove(asset)}
          >
            <Delete theme='outline' size={15} fill='currentColor' strokeWidth={3} />
          </button>
        ) : null}
      </div>
    </article>
  );
};

const AssetSkeletons: React.FC<{ view: CreativeAssetViewMode }> = ({ view }) => (
  <div className={view === 'grid' ? styles.assetGrid : styles.assetList} aria-hidden='true'>
    {Array.from({ length: view === 'grid' ? 8 : 5 }, (_, index) => (
      <div key={index} className={classNames(styles.skeleton, view === 'list' && styles.skeletonRow)}>
        <span />
        <i />
        <i />
      </div>
    ))}
  </div>
);

const CreativeAssetLibrary: React.FC<CreativeAssetLibraryProps> = ({
  state,
  search,
  kind,
  scope,
  view,
  selectedIds,
  uploads = [],
  disabled = false,
  className,
  locale,
  uploadAccept = 'image/*,video/*',
  labels: labelOverrides,
  onSearchChange,
  onKindChange,
  onScopeChange,
  onViewChange,
  onSelectionChange,
  onUploadFiles,
  onCreateText,
  onOpenAsset,
  onEditAsset,
  onDownloadAsset,
  onRemoveAsset,
  onSetSelectedLibrary,
  onInsertSelected,
  onDownloadSelected,
  onRemoveSelected,
  onCancelUpload,
  onRetryUpload,
  onDismissUpload,
}) => {
  const labels = { ...DEFAULT_CREATIVE_ASSET_LIBRARY_LABELS, ...labelOverrides };
  const inputRef = useRef<HTMLInputElement>(null);
  const [dragDepth, setDragDepth] = useState(0);
  const busy = disabled || state.mutating;
  const selectedAssets = useMemo(
    () => state.assets.filter((asset) => selectedIds.has(asset.id)),
    [state.assets, selectedIds]
  );
  const allVisibleSelected = state.assets.length > 0 && state.assets.every((asset) => selectedIds.has(asset.id));
  const filtered = search.trim().length > 0 || kind !== 'all';
  const emptyTitle = filtered
    ? labels.filteredEmptyTitle
    : scope === 'canvas'
      ? labels.canvasEmptyTitle
      : labels.emptyTitle;
  const emptyDescription = filtered
    ? labels.filteredEmptyDescription
    : scope === 'canvas'
      ? labels.canvasEmptyDescription
      : labels.emptyDescription;
  const visibleState = state.loading && state.assets.length === 0
    ? 'loading'
    : state.error && state.assets.length === 0
      ? 'error'
      : state.assets.length === 0
        ? 'empty'
        : 'ready';

  const toggleAsset = (assetId: string) => {
    const next = new Set(selectedIds);
    if (next.has(assetId)) next.delete(assetId);
    else next.add(assetId);
    onSelectionChange(next);
  };

  const selectAllVisible = () => {
    const next = new Set(selectedIds);
    if (allVisibleSelected) state.assets.forEach((asset) => next.delete(asset.id));
    else state.assets.forEach((asset) => next.add(asset.id));
    onSelectionChange(next);
  };

  const acceptFiles = (files: FileList | null) => {
    const next = files ? Array.from(files) : [];
    if (next.length) onUploadFiles?.(next);
  };

  return (
    <section
      className={classNames(styles.root, className)}
      data-creative-asset-library
      data-asset-view={view}
      data-asset-scope={scope}
      data-asset-state={visibleState}
      aria-busy={state.loading || state.loadingMore || state.mutating}
      onDragEnter={(event) => {
        if (!onUploadFiles || !event.dataTransfer.types.includes('Files')) return;
        event.preventDefault();
        setDragDepth((depth) => depth + 1);
      }}
      onDragOver={(event) => {
        if (!onUploadFiles || !event.dataTransfer.types.includes('Files')) return;
        event.preventDefault();
        event.dataTransfer.dropEffect = 'copy';
      }}
      onDragLeave={(event) => {
        if (!onUploadFiles || !event.dataTransfer.types.includes('Files')) return;
        event.preventDefault();
        setDragDepth((depth) => Math.max(0, depth - 1));
      }}
      onDrop={(event) => {
        if (!onUploadFiles) return;
        event.preventDefault();
        setDragDepth(0);
        acceptFiles(event.dataTransfer.files);
      }}
    >
      <header className={styles.header}>
        <div className={styles.titleBlock}>
          <h1>{labels.title}</h1>
          <p>{labels.description}</p>
        </div>
        <div className={styles.primaryActions}>
          {onCreateText ? (
            <button type='button' className={styles.secondaryButton} disabled={busy} onClick={onCreateText}>
              <Plus theme='outline' size={16} fill='currentColor' strokeWidth={3} />
              <span>{labels.createText}</span>
            </button>
          ) : null}
          {onUploadFiles ? (
            <button type='button' className={styles.primaryButton} disabled={busy} onClick={() => inputRef.current?.click()}>
              <Upload theme='outline' size={16} fill='currentColor' strokeWidth={3} />
              <span>{labels.upload}</span>
            </button>
          ) : null}
        </div>
      </header>

      <div className={styles.toolbar}>
        <label className={styles.searchBox}>
          <Search theme='outline' size={17} fill='currentColor' strokeWidth={3} />
          <input
            type='search'
            value={search}
            placeholder={labels.searchPlaceholder}
            disabled={disabled}
            onChange={(event) => onSearchChange(event.target.value)}
          />
          {search ? (
            <button type='button' aria-label={labels.clearSearch} onClick={() => onSearchChange('')}>
              <Close theme='outline' size={14} fill='currentColor' strokeWidth={3} />
            </button>
          ) : null}
        </label>

        <div className={styles.scopeSwitch} role='group' aria-label={labels.scopeFilter}>
          {(['library', 'canvas'] as const).map((value) => (
            <button
              key={value}
              type='button'
              aria-pressed={scope === value}
              disabled={disabled}
              onClick={() => onScopeChange(value)}
            >
              {value === 'library' ? labels.libraryScope : labels.canvasScope}
            </button>
          ))}
        </div>

        <div className={styles.viewSwitch} role='group' aria-label={labels.viewFilter}>
          <button
            type='button'
            aria-label={labels.gridView}
            title={labels.gridView}
            aria-pressed={view === 'grid'}
            onClick={() => onViewChange('grid')}
          >
            <GridNine theme='outline' size={16} fill='currentColor' strokeWidth={3} />
          </button>
          <button
            type='button'
            aria-label={labels.listView}
            title={labels.listView}
            aria-pressed={view === 'list'}
            onClick={() => onViewChange('list')}
          >
            <List theme='outline' size={16} fill='currentColor' strokeWidth={3} />
          </button>
        </div>
      </div>

      <div className={styles.filterRow}>
        <div className={styles.kindFilters} role='group' aria-label={labels.kindFilter}>
          {KIND_FILTERS.map((option) => (
            <button
              key={option.value}
              type='button'
              aria-pressed={kind === option.value}
              disabled={disabled}
              onClick={() => onKindChange(option.value)}
            >
              <span aria-hidden='true'>{option.icon}</span>
              {labels[option.value]}
            </button>
          ))}
        </div>
        <span className={styles.resultCount}>{labels.resultCount(state.assets.length, state.total)}</span>
      </div>

      {selectedAssets.length > 0 ? (
        <div className={styles.selectionBar} data-asset-selection-bar>
          <strong>{labels.selectedCount(selectedAssets.length)}</strong>
          <button type='button' className={styles.textButton} disabled={busy} onClick={selectAllVisible}>
            {allVisibleSelected ? labels.clearSelection : labels.selectAll}
          </button>
          <div className={styles.selectionActions}>
            {onSetSelectedLibrary ? (
              <button
                type='button'
                disabled={busy}
                onClick={() => onSetSelectedLibrary(selectedAssets, scope !== 'library')}
              >
                {scope === 'library' ? labels.removeFromLibrary : labels.addToLibrary}
              </button>
            ) : null}
            {onInsertSelected ? (
              <button type='button' disabled={busy} onClick={() => onInsertSelected(selectedAssets)}>
                <Plus theme='outline' size={14} fill='currentColor' strokeWidth={3} />
                {labels.insertIntoCanvas}
              </button>
            ) : null}
            {onDownloadSelected ? (
              <button type='button' disabled={busy} onClick={() => onDownloadSelected(selectedAssets)}>
                <Download theme='outline' size={14} fill='currentColor' strokeWidth={3} />
                {labels.downloadSelected}
              </button>
            ) : null}
            {onRemoveSelected ? (
              <button type='button' data-danger disabled={busy} onClick={() => onRemoveSelected(selectedAssets)}>
                <Delete theme='outline' size={14} fill='currentColor' strokeWidth={3} />
                {labels.deleteSelected}
              </button>
            ) : null}
            <button type='button' className={styles.iconButton} aria-label={labels.clearSelection} onClick={() => onSelectionChange(new Set())}>
              <Close theme='outline' size={14} fill='currentColor' strokeWidth={3} />
            </button>
          </div>
        </div>
      ) : null}

      {(state.error || state.mutationError) && state.assets.length > 0 ? (
        <div className={styles.inlineError} role='alert'>
          <Error theme='outline' size={16} fill='currentColor' strokeWidth={3} />
          <span>{(state.error ?? state.mutationError)?.message}</span>
          <button type='button' onClick={() => void state.reload()}>{labels.retry}</button>
        </div>
      ) : null}

      <div className={styles.content}>
        {visibleState === 'loading' ? (
          <div className={styles.loadingState} role='status' aria-label={labels.loading}>
            <span className={styles.srOnly}>{labels.loading}</span>
            <AssetSkeletons view={view} />
          </div>
        ) : visibleState === 'error' ? (
          <div className={styles.statePanel} role='alert'>
            <span className={styles.stateIcon}><Error theme='outline' size={28} fill='currentColor' strokeWidth={3} /></span>
            <strong>{state.error?.message}</strong>
            <button type='button' className={styles.secondaryButton} onClick={() => void state.reload()}>{labels.retry}</button>
          </div>
        ) : visibleState === 'empty' ? (
          <div className={styles.statePanel} data-empty-filtered={filtered || undefined}>
            <span className={styles.stateIcon} aria-hidden='true'>
              {kind === 'all' ? <AllApplication theme='outline' size={28} fill='currentColor' strokeWidth={3} /> : creativeAssetKindIcon(kind, 28)}
            </span>
            <strong>{emptyTitle}</strong>
            <p>{emptyDescription}</p>
          </div>
        ) : (
          <div className={view === 'grid' ? styles.assetGrid : styles.assetList}>
            {state.assets.map((asset) => (
              <AssetItem
                key={asset.id}
                asset={asset}
                view={view}
                selected={selectedIds.has(asset.id)}
                disabled={busy}
                locale={locale}
                labels={labels}
                onToggle={() => toggleAsset(asset.id)}
                onOpen={onOpenAsset}
                onEdit={onEditAsset}
                onDownload={onDownloadAsset}
                onRemove={onRemoveAsset}
              />
            ))}
          </div>
        )}

        {state.hasMore && visibleState === 'ready' ? (
          <div className={styles.loadMoreRow}>
            <button
              type='button'
              className={styles.secondaryButton}
              disabled={state.loadingMore}
              onClick={() => void state.loadMore()}
            >
              {state.loadingMore ? (
                <Loading theme='outline' size={15} fill='currentColor' strokeWidth={3} />
              ) : null}
              {state.loadingMore ? labels.loadingMore : labels.loadMore}
            </button>
          </div>
        ) : null}
      </div>

      <CreativeAssetUploadQueue
        items={uploads}
        labels={labels}
        onCancel={onCancelUpload}
        onRetry={onRetryUpload}
        onDismiss={onDismissUpload}
      />

      {dragDepth > 0 ? (
        <div className={styles.dropOverlay} aria-hidden='true'>
          <Upload theme='outline' size={30} fill='currentColor' strokeWidth={3} />
          <strong>{labels.dropFiles}</strong>
        </div>
      ) : null}

      {onUploadFiles ? (
        <input
          ref={inputRef}
          className={styles.fileInput}
          type='file'
          accept={uploadAccept}
          multiple
          tabIndex={-1}
          onChange={(event) => {
            acceptFiles(event.target.files);
            event.target.value = '';
          }}
        />
      ) : null}
    </section>
  );
};

export default CreativeAssetLibrary;
