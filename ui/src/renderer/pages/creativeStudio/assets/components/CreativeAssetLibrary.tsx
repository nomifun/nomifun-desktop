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
  Error,
  GridNine,
  Inbox,
  Left,
  List,
  Loading,
  Pic,
  Plus,
  Right,
  Search,
  Text,
  Upload,
  VideoTwo,
  Voice,
} from '@icon-park/react';
import classNames from 'classnames';
import React, { useId, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { CreativeAsset, CreativeAssetKind } from '../types';
import CreativeAssetMedia, { creativeAssetKindIcon } from './CreativeAssetMedia';
import CreativeAssetActionsMenu from './CreativeAssetActionsMenu';
import CreativeAssetUploadQueue from './CreativeAssetUploadQueue';
import type {
  CreativeAssetAction,
  CreativeAssetBatchAction,
  CreativeAssetKindFilter,
  CreativeAssetLibraryAppearance,
  CreativeAssetLibraryLabels,
  CreativeAssetLibraryState,
  CreativeAssetPagination,
  CreativeAssetScope,
  CreativeAssetUploadItem,
  CreativeAssetViewMode,
} from './types';
import { createCreativeAssetLibraryLabels } from './types';
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
  uploadHint?: string;
  appearance?: CreativeAssetLibraryAppearance;
  selectable?: boolean;
  pagination?: CreativeAssetPagination;
  labels?: Partial<CreativeAssetLibraryLabels>;
  onSearchChange: (value: string) => void;
  onSearchSubmit?: (value: string) => void;
  onKindChange: (kind: CreativeAssetKindFilter) => void;
  onScopeChange: (scope: CreativeAssetScope) => void;
  onViewChange: (view: CreativeAssetViewMode) => void;
  onSelectionChange: (selectedIds: ReadonlySet<string>) => void;
  onUploadFiles?: (files: readonly File[]) => void;
  onCreateText?: () => void;
  onRenameCollection?: () => void;
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

const SOURCE_KIND_ORDER: CreativeAssetKindFilter[] = ['all', 'text', 'image', 'video', 'audio'];

export function submitCreativeAssetLibrarySearch(
  event: Pick<React.FormEvent<HTMLFormElement>, 'preventDefault'>,
  search: string,
  onSearchSubmit?: (value: string) => void
): void {
  event.preventDefault();
  onSearchSubmit?.(search);
}

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
  selectable: boolean;
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
  selectable,
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
      {selectable ? (
        <label className={styles.assetSelect} title={labels.select}>
          <input type='checkbox' checked={selected} disabled={disabled} onChange={onToggle} />
          <span aria-hidden='true'>
            <Check theme='outline' size={13} fill='currentColor' strokeWidth={4} />
          </span>
          <span className={styles.srOnly}>{labels.select}</span>
        </label>
      ) : null}

      <div className={styles.assetCover}>
        <button
          type='button'
          className={styles.assetPreviewButton}
          disabled={!onOpen || disabled}
          aria-label={`${labels.open}: ${asset.title}`}
          onClick={() => onOpen?.(asset)}
        >
          <CreativeAssetMedia asset={asset} unavailableLabel={labels.mediaUnavailable} compact={view === 'list'} />
        </button>
        <span className={styles.kindBadge} data-kind={asset.kind}>
          <span aria-hidden='true'>{creativeAssetKindIcon(asset.kind, 13)}</span>
          {kindLabel(asset.kind, labels)}
        </span>
      </div>

      <div className={styles.assetContent}>
        <div className={styles.assetTitleBlock}>
          <strong title={asset.title}>{asset.title}</strong>
          <span title={asset.collection || labels.noCollection}>{asset.collection || labels.noCollection}</span>
        </div>

        <div className={styles.assetTags} aria-label={asset.tags.length ? asset.tags.join(', ') : labels.noTags}>
          {asset.tags.length ? (
            asset.tags.slice(0, view === 'list' ? 4 : 3).map((tag) => <span key={tag} title={tag}>{tag}</span>)
          ) : (
            <span>{labels.noTags}</span>
          )}
        </div>
        {view === 'list' ? <time dateTime={updatedAt.dateTime}>{updatedAt.label}</time> : null}
      </div>

      <footer className={styles.assetFooter}>
        <p className={styles.assetDetails} title={assetDetails(asset)}>{assetDetails(asset)}</p>
        <CreativeAssetActionsMenu
          asset={asset}
          disabled={disabled}
          labels={labels}
          onOpen={onOpen}
          onEdit={onEdit}
          onDownload={onDownload}
          onRemove={onRemove}
        />
      </footer>
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
  uploadHint,
  appearance = 'default',
  selectable = true,
  pagination,
  labels: labelOverrides,
  onSearchChange,
  onSearchSubmit,
  onKindChange,
  onScopeChange,
  onViewChange,
  onSelectionChange,
  onUploadFiles,
  onCreateText,
  onRenameCollection,
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
  const { t } = useTranslation();
  const labels = { ...createCreativeAssetLibraryLabels(t), ...labelOverrides };
  const inputRef = useRef<HTMLInputElement>(null);
  const uploadHintId = useId();
  const [dragDepth, setDragDepth] = useState(0);
  const busy = disabled || state.mutating;
  const selectedAssets = useMemo(
    () => selectable ? state.assets.filter((asset) => selectedIds.has(asset.id)) : [],
    [selectable, state.assets, selectedIds]
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
  const sourceAppearance = appearance === 'source-page';
  const kindFilters = sourceAppearance
    ? SOURCE_KIND_ORDER.map((filter) => KIND_FILTERS.find((option) => option.value === filter)!)
    : KIND_FILTERS;
  const totalPages = pagination
    ? Math.max(1, Math.ceil(Math.max(0, pagination.total) / Math.max(1, pagination.pageSize)))
    : 1;

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
      data-asset-appearance={appearance}
      aria-busy={state.loading || state.loadingMore || state.mutating}
      onDragEnter={(event) => {
        if (busy || !onUploadFiles || !event.dataTransfer.types.includes('Files')) return;
        event.preventDefault();
        setDragDepth((depth) => depth + 1);
      }}
      onDragOver={(event) => {
        if (busy || !onUploadFiles || !event.dataTransfer.types.includes('Files')) return;
        event.preventDefault();
        event.dataTransfer.dropEffect = 'copy';
      }}
      onDragLeave={(event) => {
        if (busy || !onUploadFiles || !event.dataTransfer.types.includes('Files')) return;
        event.preventDefault();
        setDragDepth((depth) => Math.max(0, depth - 1));
      }}
      onDrop={(event) => {
        if (busy || !onUploadFiles) return;
        event.preventDefault();
        setDragDepth(0);
        acceptFiles(event.dataTransfer.files);
      }}
    >
      <div className={styles.frame}>
        <header className={styles.header}>
          <div className={styles.titleBlock}>
            <h1>{labels.title}</h1>
            <p>{labels.description}</p>
          </div>
          {!sourceAppearance ? (
            <div className={styles.primaryActions}>
              {onCreateText ? (
                <button type='button' className={styles.secondaryButton} disabled={busy} onClick={onCreateText}>
                  <Plus theme='outline' size={16} fill='currentColor' strokeWidth={3} />
                  <span>{labels.createText}</span>
                </button>
              ) : null}
              {onUploadFiles ? (
                <button
                  type='button'
                  className={styles.primaryButton}
                  disabled={busy}
                  title={uploadHint}
                  aria-describedby={uploadHint ? uploadHintId : undefined}
                  onClick={() => inputRef.current?.click()}
                >
                  <Upload theme='outline' size={16} fill='currentColor' strokeWidth={3} />
                  <span>{labels.upload}</span>
                </button>
              ) : null}
              {uploadHint ? <span id={uploadHintId} className={styles.srOnly}>{uploadHint}</span> : null}
            </div>
          ) : null}
        </header>

      <div className={styles.toolbar}>
        {sourceAppearance ? (
          <form
            className={styles.searchBox}
            role='search'
            onSubmit={(event) => submitCreativeAssetLibrarySearch(event, search, onSearchSubmit)}
          >
            <Search theme='outline' size={17} fill='currentColor' strokeWidth={3} />
            <input
              type='search'
              aria-label={labels.search}
              value={search}
              placeholder={labels.searchPlaceholder}
              disabled={disabled}
              onChange={(event) => onSearchChange(event.target.value)}
            />
            {search ? (
              <button type='button' className={styles.clearSearchButton} aria-label={labels.clearSearch} onClick={() => onSearchChange('')}>
                <Close theme='outline' size={14} fill='currentColor' strokeWidth={3} />
              </button>
            ) : null}
            <button
              type='submit'
              className={styles.sourceSearchButton}
              aria-label={labels.search}
              disabled={disabled}
            >
              <Search theme='outline' size={18} fill='currentColor' strokeWidth={3} />
            </button>
          </form>
        ) : (
          <>
            <label className={styles.searchBox}>
              <Search theme='outline' size={17} fill='currentColor' strokeWidth={3} />
              <input
                type='search'
                aria-label={labels.search}
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
          </>
        )}
      </div>

      <div className={styles.filterRow}>
        <div className={styles.sourceFilterGroup}>
          {sourceAppearance ? <span className={styles.sourceFilterLabel}>{labels.kindFilter}</span> : null}
          <div className={styles.kindFilters} role='group' aria-label={labels.kindFilter}>
            {kindFilters.map((option) => (
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
        </div>
        {sourceAppearance ? (
          <div className={styles.sourceActions}>
            {onRenameCollection ? (
              <button type='button' disabled={busy} onClick={onRenameCollection}>{labels.renameCollection}</button>
            ) : null}
            {onUploadFiles ? (
              <button
                type='button'
                disabled={busy}
                title={uploadHint}
                aria-describedby={uploadHint ? uploadHintId : undefined}
                onClick={() => inputRef.current?.click()}
              >
                {labels.upload}
              </button>
            ) : null}
            {onCreateText ? (
              <button type='button' disabled={busy} onClick={onCreateText}>{labels.createText}</button>
            ) : null}
            {uploadHint ? <span id={uploadHintId} className={styles.srOnly}>{uploadHint}</span> : null}
          </div>
        ) : (
          <span className={styles.resultCount}>{labels.resultCount(state.assets.length, state.total)}</span>
        )}
      </div>

      {selectable && selectedAssets.length > 0 ? (
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
              {sourceAppearance ? (
                <Inbox theme='outline' size={48} fill='currentColor' strokeWidth={2} />
              ) : kind === 'all' ? (
                <AllApplication theme='outline' size={28} fill='currentColor' strokeWidth={3} />
              ) : creativeAssetKindIcon(kind, 28)}
            </span>
            <strong>{emptyTitle}</strong>
            {emptyDescription ? <p>{emptyDescription}</p> : null}
          </div>
        ) : (
          <div className={view === 'grid' ? styles.assetGrid : styles.assetList}>
            {state.assets.map((asset) => (
              <AssetItem
                key={asset.id}
                asset={asset}
                view={view}
                selected={selectable && selectedIds.has(asset.id)}
                selectable={selectable}
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

      {sourceAppearance && pagination ? (
        <nav
          className={styles.sourcePagination}
          aria-label={labels.pagination}
          data-empty={pagination.total <= 0 || undefined}
        >
          <button
            type='button'
            aria-label={labels.previousPage}
            disabled={pagination.loading || pagination.page <= 1}
            onClick={() => pagination.onPageChange(pagination.page - 1)}
          >
            <Left theme='outline' size={13} fill='currentColor' strokeWidth={3} />
          </button>
          <span className={styles.sourcePageNumber} aria-current='page'>{pagination.page}</span>
          <button
            type='button'
            aria-label={labels.nextPage}
            disabled={pagination.loading || pagination.page >= totalPages}
            onClick={() => pagination.onPageChange(pagination.page + 1)}
          >
            {pagination.loading ? (
              <Loading theme='outline' size={13} fill='currentColor' strokeWidth={3} />
            ) : (
              <Right theme='outline' size={13} fill='currentColor' strokeWidth={3} />
            )}
          </button>
          <span className={styles.sourcePageSize}>{labels.pageSize(pagination.pageSize)}</span>
        </nav>
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
