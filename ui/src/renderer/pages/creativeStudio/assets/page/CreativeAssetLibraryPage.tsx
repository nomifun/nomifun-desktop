/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Button, Checkbox, Input, InputTag, Message, Modal } from '@arco-design/web-react';
import type { TFunction } from 'i18next';
import React, { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { isBackendHttpError } from '@/common/adapter/httpBridge';
import { creativeAssetClient } from '../client';
import CreativeVideoPlayer from '../components/CreativeVideoPlayer';
import { subscribeCreativeAssetDeletion } from '../assetDeletion';
import {
  CreateCreativeTextAssetModal,
  CreativeAssetLibrary,
} from '../components';
import type {
  CreativeAssetKindFilter,
  CreativeAssetViewMode,
  CreativeTextAssetFormValue,
} from '../components';
import { isCreativeAssetDeleted, type CreativeAsset, type CreativeAssetLibraryPort } from '../types';
import { useCreativeAssets } from '../useCreativeAssets';
import styles from './CreativeAssetLibraryPage.module.css';
import {
  CREATIVE_ASSET_MANUAL_UPLOAD_ACCEPT,
  EMPTY_CREATIVE_TEXT_ASSET_FORM,
  buildGlobalCreativeAssetQuery,
  creativeAssetDownloadName,
  creativeAssetEditDraft,
  creativeAssetPageCount,
  creativeAssetPageIsLoaded,
  creativeAssetPageSlice,
  creativeAssetQuerySearch,
  manualUploadRejectionMessage,
  normalizeCreativeAssetEditDraft,
  normalizeCreativeTextAssetForm,
  validateCreativeAssetManualUpload,
  validateCreativeCollectionRename,
} from './model';
import type { CreativeAssetEditDraft, CreativeCollectionRenameDraft } from './model';
import { useCreativeAssetUploadQueue } from './useCreativeAssetUploadQueue';

const EMPTY_SELECTION = new Set<string>();
const SOURCE_ASSET_PAGE_SIZE = 10;
const DEFAULT_EDIT_DRAFT: CreativeAssetEditDraft = {
  title: '',
  collection: '',
  tags: [],
  inLibrary: true,
};
const DEFAULT_RENAME_DRAFT: CreativeCollectionRenameDraft = { from: '', to: '' };

const popupContainer = (): HTMLElement =>
  document.getElementById('creative-studio-portal-root') ?? document.body;

const errorText = (reason: unknown): string =>
  reason instanceof Error ? reason.message : String(reason);

const assetKindLabel = (t: TFunction, kind: CreativeAsset['kind']): string => {
  switch (kind) {
    case 'image':
      return t('creativeStudio.assets.kind.image', { defaultValue: '图片' });
    case 'video':
      return t('creativeStudio.assets.kind.video', { defaultValue: '视频' });
    case 'audio':
      return t('creativeStudio.assets.kind.audio', { defaultValue: '音频' });
    case 'text':
      return t('creativeStudio.assets.kind.text', { defaultValue: '文本' });
  }
};

function useDebouncedValue<T>(value: T, delay: number): T {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const timer = window.setTimeout(() => setDebounced(value), delay);
    return () => window.clearTimeout(timer);
  }, [delay, value]);
  return debounced;
}

const downloadAsset = (asset: CreativeAsset): void => {
  if (isCreativeAssetDeleted(asset)) return;
  const anchor = document.createElement('a');
  anchor.href = asset.originalUrl;
  anchor.download = creativeAssetDownloadName(asset);
  anchor.rel = 'noopener noreferrer';
  anchor.click();
};

interface AssetPreviewModalProps {
  asset: CreativeAsset | null;
  onClose: () => void;
}

const AssetPreviewModal: React.FC<AssetPreviewModalProps> = ({ asset, onClose }) => {
  const { t } = useTranslation();
  return (
    <Modal
      visible={Boolean(asset)}
      title={
        asset?.title ??
        t('creativeStudio.assets.preview.title', { defaultValue: '素材详情' })
      }
      footer={null}
      autoFocus={false}
      focusLock
      unmountOnExit
      className={styles.modalBody}
      getPopupContainer={popupContainer}
      onCancel={onClose}
    >
      {asset ? (
        <div className={styles.previewBody} data-creative-asset-preview={asset.kind}>
          <div className={styles.previewMedia}>
            {isCreativeAssetDeleted(asset) ? (
              <p role='status'>{t('creativeStudio.assets.deleted', { defaultValue: '素材已删除' })}</p>
            ) : asset.kind === 'image' ? (
              <img src={asset.originalUrl} alt={asset.title} />
            ) : asset.kind === 'video' ? (
              <div className={styles.previewVideo}>
                <CreativeVideoPlayer src={asset.originalUrl} poster={asset.thumbnailUrl ?? undefined} label={asset.title} />
              </div>
            ) : asset.kind === 'audio' ? (
              <audio src={asset.originalUrl} controls preload='metadata' aria-label={asset.title} />
            ) : (
              <pre className={styles.previewText}>{asset.textContent ?? ''}</pre>
            )}
          </div>
          <div className={styles.previewMeta}>
            <p>
              {t('creativeStudio.assets.preview.kind', {
                defaultValue: '类型：{{kind}}',
                kind: assetKindLabel(t, asset.kind),
              })}
            </p>
            <p>
              {t('creativeStudio.assets.preview.collection', {
                defaultValue: '合集：{{collection}}',
                collection:
                  asset.collection ||
                  t('creativeStudio.assets.library.noCollection', { defaultValue: '未分组' }),
              })}
            </p>
            {asset.mimeType ? (
              <p>
                {t('creativeStudio.assets.preview.mime', {
                  defaultValue: 'MIME：{{mime}}',
                  mime: asset.mimeType,
                })}
              </p>
            ) : null}
          </div>
          {asset.tags.length ? (
            <div
              className={styles.previewTags}
              aria-label={t('creativeStudio.assets.preview.tags', { defaultValue: '素材标签' })}
            >
              {asset.tags.map((tag) => <span key={tag}>{tag}</span>)}
            </div>
          ) : null}
          <footer className={styles.previewFooter}>
            {asset.kind !== 'text' ? (
              <Button type='primary' disabled={isCreativeAssetDeleted(asset)} onClick={() => downloadAsset(asset)}>
                {t('creativeStudio.assets.preview.downloadOriginal', {
                  defaultValue: '下载原始文件',
                })}
              </Button>
            ) : null}
            <Button onClick={onClose}>
              {t('creativeStudio.assets.preview.close', { defaultValue: '关闭' })}
            </Button>
          </footer>
        </div>
      ) : null}
    </Modal>
  );
};

interface EditAssetModalProps {
  asset: CreativeAsset | null;
  draft: CreativeAssetEditDraft;
  submitting: boolean;
  error: string | null;
  onDraftChange: (draft: CreativeAssetEditDraft) => void;
  onCancel: () => void;
  onSubmit: () => void;
}

const EditAssetModal: React.FC<EditAssetModalProps> = ({
  asset,
  draft,
  submitting,
  error,
  onDraftChange,
  onCancel,
  onSubmit,
}) => {
  const { t } = useTranslation();
  const valid = draft.title.trim().length > 0;
  const patch = (next: Partial<CreativeAssetEditDraft>) => onDraftChange({ ...draft, ...next });
  return (
    <Modal
      visible={Boolean(asset)}
      title={t('creativeStudio.assets.edit.title', { defaultValue: '编辑素材' })}
      footer={null}
      autoFocus={false}
      focusLock
      unmountOnExit
      maskClosable={!submitting}
      closable={!submitting}
      getPopupContainer={popupContainer}
      onCancel={() => {
        if (!submitting) onCancel();
      }}
    >
      <form
        className={styles.modalForm}
        data-edit-creative-asset-form
        onSubmit={(event) => {
          event.preventDefault();
          if (valid && !submitting) onSubmit();
        }}
      >
        <p className={styles.modalDescription}>
          {t('creativeStudio.assets.edit.description', {
            defaultValue:
              '可修改后端支持的标题、合集、标签和素材库状态；素材类型与原始文件不可替换。',
          })}
        </p>
        <label className={styles.field}>
          <span>{t('creativeStudio.assets.edit.titleLabel', { defaultValue: '标题' })}</span>
          <Input value={draft.title} maxLength={240} disabled={submitting} onChange={(title) => patch({ title })} />
        </label>
        <label className={styles.field}>
          <span>{t('creativeStudio.assets.edit.collectionLabel', { defaultValue: '合集' })}</span>
          <Input
            value={draft.collection}
            maxLength={240}
            placeholder={t('creativeStudio.assets.edit.collectionPlaceholder', {
              defaultValue: '留空表示未分组',
            })}
            disabled={submitting}
            onChange={(collection) => patch({ collection })}
          />
        </label>
        <label className={styles.field}>
          <span>{t('creativeStudio.assets.edit.tagsLabel', { defaultValue: '标签' })}</span>
          <InputTag
            value={draft.tags}
            allowClear
            placeholder={t('creativeStudio.assets.edit.tagsPlaceholder', {
              defaultValue: '输入标签后按回车',
            })}
            disabled={submitting}
            onChange={(tags) => patch({ tags: tags.map(String) })}
          />
        </label>
        <Checkbox checked={draft.inLibrary} disabled={submitting} onChange={(inLibrary) => patch({ inLibrary })}>
          {t('creativeStudio.assets.edit.keepInLibrary', { defaultValue: '保留在素材库' })}
        </Checkbox>
        {!valid ? (
          <p className={styles.modalError}>
            {t('creativeStudio.assets.edit.titleRequired', { defaultValue: '标题不能为空。' })}
          </p>
        ) : null}
        {error ? <p className={styles.modalError} role='alert'>{error}</p> : null}
        <footer className={styles.modalFooter}>
          <Button disabled={submitting} onClick={onCancel}>
            {t('creativeStudio.assets.edit.cancel', { defaultValue: '取消' })}
          </Button>
          <Button type='primary' htmlType='submit' loading={submitting} disabled={!valid || submitting}>
            {t('creativeStudio.assets.edit.save', { defaultValue: '保存' })}
          </Button>
        </footer>
      </form>
    </Modal>
  );
};

export interface CreativeAssetLibraryPageProps {
  client?: CreativeAssetLibraryPort;
  locale?: string;
}

const CreativeAssetLibraryPage: React.FC<CreativeAssetLibraryPageProps> = ({
  client = creativeAssetClient,
  locale,
}) => {
  const { t, i18n } = useTranslation();
  const [search, setSearch] = useState('');
  const debouncedSearch = useDebouncedValue(search, 220);
  const [submittedSearch, setSubmittedSearch] = useState<string | null>(null);
  const [kind, setKind] = useState<CreativeAssetKindFilter>('all');
  const [view, setView] = useState<CreativeAssetViewMode>('grid');
  const [page, setPage] = useState(1);
  const querySearch = creativeAssetQuerySearch(debouncedSearch, submittedSearch);

  const query = useMemo(
    () => buildGlobalCreativeAssetQuery(querySearch, kind),
    [kind, querySearch]
  );
  const library = useCreativeAssets({ client, query, pageSize: SOURCE_ASSET_PAGE_SIZE });
  const uploads = useCreativeAssetUploadQueue(library.upload);
  const pageLoadAttemptRef = useRef<{ page: number; loaded: number } | null>(null);
  const totalPages = creativeAssetPageCount(library.total, SOURCE_ASSET_PAGE_SIZE);
  const visiblePage = Math.min(page, totalPages);
  const pageHasLoadedAssets = creativeAssetPageIsLoaded(
    library.assets.length,
    library.total,
    visiblePage,
    SOURCE_ASSET_PAGE_SIZE
  );
  const pageLoaded = !library.loading
    && pageHasLoadedAssets
    && (!library.error || library.assets.length > 0);

  useEffect(() => {
    setPage(1);
    pageLoadAttemptRef.current = null;
  }, [kind, querySearch]);

  useEffect(() => {
    setPage((current) => Math.min(current, totalPages));
  }, [totalPages]);

  useEffect(() => {
    if (pageLoaded) {
      pageLoadAttemptRef.current = null;
      return;
    }
    if (library.error) {
      pageLoadAttemptRef.current = null;
      return;
    }
    if (library.loading || library.loadingMore || !library.hasMore) return;
    const previousAttempt = pageLoadAttemptRef.current;
    if (
      previousAttempt?.page === visiblePage &&
      previousAttempt.loaded === library.assets.length
    ) {
      return;
    }
    pageLoadAttemptRef.current = { page: visiblePage, loaded: library.assets.length };
    void library.loadMore();
  }, [
    library.assets.length,
    library.error,
    library.hasMore,
    library.loadMore,
    library.loading,
    library.loadingMore,
    pageLoaded,
    visiblePage,
  ]);

  const visibleAssets = pageLoaded
    ? creativeAssetPageSlice(library.assets, visiblePage, SOURCE_ASSET_PAGE_SIZE)
    : [];
  const pageState = {
    ...library,
    assets: visibleAssets,
    loading: !library.error && !pageLoaded,
    loadingMore: library.loadingMore,
    mutating: library.mutating,
    error: pageLoaded ? null : library.error,
    hasMore: false,
  };

  const [textModalOpen, setTextModalOpen] = useState(false);
  const [textDraft, setTextDraft] = useState<CreativeTextAssetFormValue>(EMPTY_CREATIVE_TEXT_ASSET_FORM);
  const [textSubmitting, setTextSubmitting] = useState(false);
  const [textError, setTextError] = useState<string | null>(null);

  const [previewAsset, setPreviewAsset] = useState<CreativeAsset | null>(null);
  const [editingAsset, setEditingAsset] = useState<CreativeAsset | null>(null);

  useEffect(() => subscribeCreativeAssetDeletion(client, (assetId) => {
    setPreviewAsset((current) => current?.id === assetId
      ? { ...current, deletedAt: Date.now(), textContent: null, originalUrl: '', thumbnailUrl: null, inLibrary: false }
      : current);
    setEditingAsset((current) => current?.id === assetId ? null : current);
  }), [client]);

  useEffect(() => {
    const reader = client as CreativeAssetLibraryPort & { get?(id: string): Promise<CreativeAsset> };
    if (!previewAsset || !reader.get) return;
    let active = true;
    const id = previewAsset.id;
    const refresh = () => {
      void reader.get!(id).then((asset) => {
        if (active) setPreviewAsset(asset);
      }).catch(() => undefined);
    };
    refresh();
    window.addEventListener('focus', refresh);
    return () => { active = false; window.removeEventListener('focus', refresh); };
  }, [client, previewAsset?.id]);
  const [editDraft, setEditDraft] = useState<CreativeAssetEditDraft>(DEFAULT_EDIT_DRAFT);
  const [editSubmitting, setEditSubmitting] = useState(false);
  const [editError, setEditError] = useState<string | null>(null);

  const [deletingAsset, setDeletingAsset] = useState<CreativeAsset | null>(null);
  const [deleteSubmitting, setDeleteSubmitting] = useState(false);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const deleteSubmittingRef = useRef(false);

  const [renameOpen, setRenameOpen] = useState(false);
  const [renameDraft, setRenameDraft] = useState<CreativeCollectionRenameDraft>(DEFAULT_RENAME_DRAFT);
  const [renameSubmitting, setRenameSubmitting] = useState(false);
  const [renameError, setRenameError] = useState<string | null>(null);

  const handleUploadFiles = (files: readonly File[]): void => {
    const accepted: File[] = [];
    const rejections = new Set<string>();
    for (const file of files) {
      const result = validateCreativeAssetManualUpload(file);
      if (result.accepted) accepted.push(file);
      else if (result.rejection) {
        rejections.add(manualUploadRejectionMessage(result.rejection, t));
      }
    }
    if (rejections.size) Message.warning([...rejections].join(' '));
    if (accepted.length) uploads.start(accepted);
  };

  const handleCreateText = async (): Promise<void> => {
    const input = normalizeCreativeTextAssetForm(textDraft);
    if (!input.title || !input.textContent) return;
    setTextSubmitting(true);
    setTextError(null);
    try {
      await library.createText({
        title: input.title,
        textContent: input.textContent,
        collection: input.collection || undefined,
        tags: input.tags,
        inLibrary: input.inLibrary,
      });
      setTextModalOpen(false);
      setTextDraft(EMPTY_CREATIVE_TEXT_ASSET_FORM);
      Message.success(
        t('creativeStudio.assets.messages.textCreated', { defaultValue: '文本素材已创建。' })
      );
    } catch (reason) {
      setTextError(errorText(reason));
    } finally {
      setTextSubmitting(false);
    }
  };

  const openEdit = (asset: CreativeAsset): void => {
    setEditingAsset(asset);
    setEditDraft(creativeAssetEditDraft(asset));
    setEditError(null);
  };

  const handleEdit = async (): Promise<void> => {
    if (!editingAsset) return;
    const draft = normalizeCreativeAssetEditDraft(editDraft);
    if (!draft.title) return;
    setEditSubmitting(true);
    setEditError(null);
    try {
      await library.update(editingAsset.id, {
        title: draft.title,
        collection: draft.collection || null,
        tags: draft.tags,
        inLibrary: draft.inLibrary,
      });
      void library.reload();
      setEditingAsset(null);
      Message.success(
        t('creativeStudio.assets.messages.assetUpdated', { defaultValue: '素材已更新。' })
      );
    } catch (reason) {
      setEditError(errorText(reason));
    } finally {
      setEditSubmitting(false);
    }
  };

  const handleDelete = async (): Promise<void> => {
    if (!deletingAsset || deleteSubmittingRef.current) return;
    deleteSubmittingRef.current = true;
    setDeleteSubmitting(true);
    setDeleteError(null);
    try {
      await library.remove(deletingAsset.id);
      void library.reload();
      setDeletingAsset(null);
      Message.success(
        t('creativeStudio.assets.messages.assetDeleted', { defaultValue: '素材已删除。' })
      );
    } catch (reason) {
      setDeleteError(
        isBackendHttpError(reason) && reason.status === 409
          ? t('creativeStudio.assets.delete.activeTask', {
              defaultValue: '素材仍被正在执行的生成任务使用，请等待任务结束或取消任务后再删除。',
            })
          : isBackendHttpError(reason) && reason.status >= 500
            ? t('creativeStudio.assets.delete.retryCleanup', { defaultValue: '删除或文件清理尚未完成，请重试删除。' })
            : isBackendHttpError(reason) ? reason.backendMessage : errorText(reason)
      );
    } finally {
      deleteSubmittingRef.current = false;
      setDeleteSubmitting(false);
    }
  };

  const handleRenameCollection = async (): Promise<void> => {
    const validation = validateCreativeCollectionRename(renameDraft, t);
    if (validation) {
      setRenameError(validation);
      return;
    }
    setRenameSubmitting(true);
    setRenameError(null);
    try {
      const updated = await library.renameCollection(renameDraft.from.trim(), renameDraft.to.trim());
      setRenameOpen(false);
      setRenameDraft(DEFAULT_RENAME_DRAFT);
      if (updated > 0) {
        Message.success(
          t('creativeStudio.assets.messages.collectionUpdated', {
            defaultValue: '已更新 {{assetCount}} 个素材。',
            assetCount: updated,
          })
        );
      } else {
        Message.info(
          t('creativeStudio.assets.messages.collectionUnused', {
            defaultValue: '没有找到使用该合集的素材。',
          })
        );
      }
    } catch (reason) {
      setRenameError(errorText(reason));
    } finally {
      setRenameSubmitting(false);
    }
  };

  const openRenameCollection = (): void => {
    setRenameError(null);
    setRenameOpen(true);
  };

  const handlePageChange = (nextPage: number): void => {
    if (nextPage < 1 || nextPage > totalPages) return;
    setPage(nextPage);
  };

  return (
    <main className={styles.root} data-creative-asset-library-page>
      <CreativeAssetLibrary
        className={styles.library}
        appearance='source-page'
        selectable={false}
        state={pageState}
        search={search}
        kind={kind}
        scope='library'
        view={view}
        locale={locale ?? i18n.resolvedLanguage ?? i18n.language}
        selectedIds={EMPTY_SELECTION}
        uploads={uploads.items}
        uploadAccept={CREATIVE_ASSET_MANUAL_UPLOAD_ACCEPT}
        uploadHint={t('creativeStudio.assets.upload.hint', {
          defaultValue: '支持图片和视频，单文件最大 64 MB；暂不支持手动上传音频。',
        })}
        pagination={{
          page: visiblePage,
          pageSize: SOURCE_ASSET_PAGE_SIZE,
          total: library.total,
          loading: library.loading || library.loadingMore || (!pageLoaded && !library.error),
          onPageChange: handlePageChange,
        }}
        labels={{
          title: t('creativeStudio.assets.page.title', { defaultValue: '我的素材' }),
          description: t('creativeStudio.assets.page.description', {
            defaultValue: '收藏常用素材，按类型和标题快速查找。',
          }),
          searchPlaceholder: t('creativeStudio.assets.page.searchPlaceholder', {
            defaultValue: '搜索素材标题',
          }),
          kindFilter: t('creativeStudio.assets.page.kindFilter', { defaultValue: '类型' }),
          emptyTitle: t('creativeStudio.assets.page.emptyTitle', {
            defaultValue: '没有找到素材',
          }),
          emptyDescription: '',
          filteredEmptyTitle: t('creativeStudio.assets.page.filteredEmptyTitle', {
            defaultValue: '没有找到素材',
          }),
          filteredEmptyDescription: '',
        }}
        onSearchChange={(value) => {
          setSearch(value);
          setSubmittedSearch(null);
        }}
        onSearchSubmit={(value) => {
          setSearch(value);
          setSubmittedSearch(value);
          setPage(1);
        }}
        onKindChange={setKind}
        onScopeChange={() => undefined}
        onViewChange={setView}
        onSelectionChange={() => undefined}
        onUploadFiles={handleUploadFiles}
        onCreateText={() => {
          setTextError(null);
          setTextDraft(EMPTY_CREATIVE_TEXT_ASSET_FORM);
          setTextModalOpen(true);
        }}
        onRenameCollection={openRenameCollection}
        onOpenAsset={setPreviewAsset}
        onEditAsset={openEdit}
        onDownloadAsset={downloadAsset}
        onRemoveAsset={(asset) => {
          if (deleteSubmittingRef.current) return;
          setDeleteError(null);
          setDeletingAsset(asset);
        }}
        onCancelUpload={uploads.cancel}
        onRetryUpload={uploads.retry}
        onDismissUpload={uploads.dismiss}
      />

      <CreateCreativeTextAssetModal
        open={textModalOpen}
        value={textDraft}
        submitting={textSubmitting}
        error={textError}
        onChange={setTextDraft}
        onCancel={() => {
          if (!textSubmitting) setTextModalOpen(false);
        }}
        onSubmit={() => void handleCreateText()}
      />

      <AssetPreviewModal asset={previewAsset} onClose={() => setPreviewAsset(null)} />

      <EditAssetModal
        asset={editingAsset}
        draft={editDraft}
        submitting={editSubmitting}
        error={editError}
        onDraftChange={setEditDraft}
        onCancel={() => setEditingAsset(null)}
        onSubmit={() => void handleEdit()}
      />

      <Modal
        visible={Boolean(deletingAsset)}
        title={t('creativeStudio.assets.delete.title', { defaultValue: '删除素材' })}
        confirmLoading={deleteSubmitting}
        okButtonProps={{ status: 'danger' }}
        okText={t('creativeStudio.assets.delete.confirm', { defaultValue: '永久删除' })}
        cancelText={t('creativeStudio.assets.delete.cancel', { defaultValue: '取消' })}
        maskClosable={!deleteSubmitting}
        closable={!deleteSubmitting}
        getPopupContainer={popupContainer}
        onOk={() => void handleDelete()}
        onCancel={() => {
          if (!deleteSubmittingRef.current) setDeletingAsset(null);
        }}
      >
        <div className={styles.modalBody}>
          <p className={styles.deleteText}>
            {t('creativeStudio.assets.delete.description', {
              defaultValue: '确定永久删除“{{title}}”吗？原始文件及缩略图将被删除，且无法恢复。使用此素材的画布和生成历史会保留记录，并显示“素材已删除”。',
              title: deletingAsset?.title ?? '',
            })}
          </p>
          {deleteError ? <p className={styles.modalError} role='alert'>{deleteError}</p> : null}
        </div>
      </Modal>

      <Modal
        visible={renameOpen}
        title={t('creativeStudio.assets.collection.renameTitle', {
          defaultValue: '重命名合集',
        })}
        footer={null}
        autoFocus={false}
        focusLock
        unmountOnExit
        maskClosable={!renameSubmitting}
        closable={!renameSubmitting}
        getPopupContainer={popupContainer}
        onCancel={() => {
          if (!renameSubmitting) setRenameOpen(false);
        }}
      >
        <form
          className={styles.modalForm}
          data-rename-creative-asset-collection-form
          onSubmit={(event) => {
            event.preventDefault();
            if (!renameSubmitting) void handleRenameCollection();
          }}
        >
          <p className={styles.modalDescription}>
            {t('creativeStudio.assets.collection.renameDescription', {
              defaultValue:
                '这会更新所有使用当前合集名称的素材。新名称留空会将这些素材设为未分组。',
            })}
          </p>
          <label className={styles.field}>
            <span>
              {t('creativeStudio.assets.collection.currentNameLabel', {
                defaultValue: '当前合集名称',
              })}
            </span>
            <Input
              value={renameDraft.from}
              maxLength={240}
              disabled={renameSubmitting}
              onChange={(from) => setRenameDraft((draft) => ({ ...draft, from }))}
            />
          </label>
          <label className={styles.field}>
            <span>
              {t('creativeStudio.assets.collection.newNameLabel', {
                defaultValue: '新合集名称',
              })}
            </span>
            <Input
              value={renameDraft.to}
              maxLength={240}
              placeholder={t('creativeStudio.assets.collection.newNamePlaceholder', {
                defaultValue: '留空表示取消分组',
              })}
              disabled={renameSubmitting}
              onChange={(to) => setRenameDraft((draft) => ({ ...draft, to }))}
            />
          </label>
          {renameError ? <p className={styles.modalError} role='alert'>{renameError}</p> : null}
          <footer className={styles.modalFooter}>
            <Button disabled={renameSubmitting} onClick={() => setRenameOpen(false)}>
              {t('creativeStudio.assets.collection.cancel', { defaultValue: '取消' })}
            </Button>
            <Button type='primary' htmlType='submit' loading={renameSubmitting}>
              {t('creativeStudio.assets.collection.confirm', { defaultValue: '确认更新' })}
            </Button>
          </footer>
        </form>
      </Modal>
    </main>
  );
};

export default CreativeAssetLibraryPage;
