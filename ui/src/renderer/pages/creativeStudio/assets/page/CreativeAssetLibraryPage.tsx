/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Button, Checkbox, Input, InputTag, Message, Modal } from '@arco-design/web-react';
import React, { useEffect, useMemo, useState } from 'react';

import { creativeAssetClient } from '../client';
import {
  CreateCreativeTextAssetModal,
  CreativeAssetLibrary,
} from '../components';
import type {
  CreativeAssetKindFilter,
  CreativeAssetScope,
  CreativeAssetViewMode,
  CreativeTextAssetFormValue,
} from '../components';
import type { CreativeAsset, CreativeAssetLibraryPort } from '../types';
import { useCreativeAssets } from '../useCreativeAssets';
import styles from './CreativeAssetLibraryPage.module.css';
import {
  CREATIVE_ASSET_MANUAL_UPLOAD_ACCEPT,
  EMPTY_CREATIVE_TEXT_ASSET_FORM,
  buildGlobalCreativeAssetQuery,
  creativeAssetDownloadName,
  creativeAssetEditDraft,
  manualUploadRejectionMessage,
  normalizeCreativeAssetEditDraft,
  normalizeCreativeTextAssetForm,
  validateCreativeAssetManualUpload,
  validateCreativeCollectionRename,
} from './model';
import type { CreativeAssetEditDraft, CreativeCollectionRenameDraft } from './model';
import { useCreativeAssetUploadQueue } from './useCreativeAssetUploadQueue';

const GLOBAL_SCOPE_ARIA_LABEL = 'creative-studio-global-scope-fixed';
const EMPTY_SELECTION = new Set<string>();
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

function useDebouncedValue<T>(value: T, delay: number): T {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const timer = window.setTimeout(() => setDebounced(value), delay);
    return () => window.clearTimeout(timer);
  }, [delay, value]);
  return debounced;
}

const downloadAsset = (asset: CreativeAsset): void => {
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

const AssetPreviewModal: React.FC<AssetPreviewModalProps> = ({ asset, onClose }) => (
  <Modal
    visible={Boolean(asset)}
    title={asset?.title ?? '素材详情'}
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
          {asset.kind === 'image' ? (
            <img src={asset.originalUrl} alt={asset.title} />
          ) : asset.kind === 'video' ? (
            <video src={asset.originalUrl} controls playsInline preload='metadata' aria-label={asset.title} />
          ) : asset.kind === 'audio' ? (
            <audio src={asset.originalUrl} controls preload='metadata' aria-label={asset.title} />
          ) : (
            <pre className={styles.previewText}>{asset.textContent ?? ''}</pre>
          )}
        </div>
        <div className={styles.previewMeta}>
          <p>类型：{asset.kind}</p>
          <p>合集：{asset.collection || '未分组'}</p>
          {asset.mimeType ? <p>MIME：{asset.mimeType}</p> : null}
        </div>
        {asset.tags.length ? (
          <div className={styles.previewTags} aria-label='素材标签'>
            {asset.tags.map((tag) => <span key={tag}>{tag}</span>)}
          </div>
        ) : null}
        <footer className={styles.previewFooter}>
          {asset.kind !== 'text' ? (
            <Button type='primary' onClick={() => downloadAsset(asset)}>下载原始文件</Button>
          ) : null}
          <Button onClick={onClose}>关闭</Button>
        </footer>
      </div>
    ) : null}
  </Modal>
);

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
  const valid = draft.title.trim().length > 0;
  const patch = (next: Partial<CreativeAssetEditDraft>) => onDraftChange({ ...draft, ...next });
  return (
    <Modal
      visible={Boolean(asset)}
      title='编辑素材'
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
        <p className={styles.modalDescription}>可修改后端支持的标题、合集、标签和素材库状态；素材类型与原始文件不可替换。</p>
        <label className={styles.field}>
          <span>标题</span>
          <Input value={draft.title} maxLength={240} disabled={submitting} onChange={(title) => patch({ title })} />
        </label>
        <label className={styles.field}>
          <span>合集</span>
          <Input
            value={draft.collection}
            maxLength={240}
            placeholder='留空表示未分组'
            disabled={submitting}
            onChange={(collection) => patch({ collection })}
          />
        </label>
        <label className={styles.field}>
          <span>标签</span>
          <InputTag
            value={draft.tags}
            allowClear
            placeholder='输入标签后按回车'
            disabled={submitting}
            onChange={(tags) => patch({ tags: tags.map(String) })}
          />
        </label>
        <Checkbox checked={draft.inLibrary} disabled={submitting} onChange={(inLibrary) => patch({ inLibrary })}>
          保留在素材库
        </Checkbox>
        {!valid ? <p className={styles.modalError}>标题不能为空。</p> : null}
        {error ? <p className={styles.modalError} role='alert'>{error}</p> : null}
        <footer className={styles.modalFooter}>
          <Button disabled={submitting} onClick={onCancel}>取消</Button>
          <Button type='primary' htmlType='submit' loading={submitting} disabled={!valid || submitting}>保存</Button>
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
  const [search, setSearch] = useState('');
  const debouncedSearch = useDebouncedValue(search, 220);
  const [kind, setKind] = useState<CreativeAssetKindFilter>('all');
  const [view, setView] = useState<CreativeAssetViewMode>('grid');

  const query = useMemo(
    () => buildGlobalCreativeAssetQuery(debouncedSearch, kind),
    [debouncedSearch, kind]
  );
  const library = useCreativeAssets({ client, query });
  const uploads = useCreativeAssetUploadQueue(library.upload);

  const [textModalOpen, setTextModalOpen] = useState(false);
  const [textDraft, setTextDraft] = useState<CreativeTextAssetFormValue>(EMPTY_CREATIVE_TEXT_ASSET_FORM);
  const [textSubmitting, setTextSubmitting] = useState(false);
  const [textError, setTextError] = useState<string | null>(null);

  const [previewAsset, setPreviewAsset] = useState<CreativeAsset | null>(null);
  const [editingAsset, setEditingAsset] = useState<CreativeAsset | null>(null);
  const [editDraft, setEditDraft] = useState<CreativeAssetEditDraft>(DEFAULT_EDIT_DRAFT);
  const [editSubmitting, setEditSubmitting] = useState(false);
  const [editError, setEditError] = useState<string | null>(null);

  const [deletingAsset, setDeletingAsset] = useState<CreativeAsset | null>(null);
  const [deleteSubmitting, setDeleteSubmitting] = useState(false);
  const [deleteError, setDeleteError] = useState<string | null>(null);

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
      else if (result.rejection) rejections.add(manualUploadRejectionMessage(result.rejection));
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
      Message.success('文本素材已创建。');
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
      setEditingAsset(null);
      Message.success('素材已更新。');
    } catch (reason) {
      setEditError(errorText(reason));
    } finally {
      setEditSubmitting(false);
    }
  };

  const handleDelete = async (): Promise<void> => {
    if (!deletingAsset) return;
    setDeleteSubmitting(true);
    setDeleteError(null);
    try {
      await library.remove(deletingAsset.id);
      setDeletingAsset(null);
      Message.success('素材已删除。');
    } catch (reason) {
      setDeleteError(errorText(reason));
    } finally {
      setDeleteSubmitting(false);
    }
  };

  const handleRenameCollection = async (): Promise<void> => {
    const validation = validateCreativeCollectionRename(renameDraft);
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
      if (updated > 0) Message.success(`已更新 ${updated} 个素材。`);
      else Message.info('没有找到使用该合集的素材。');
    } catch (reason) {
      setRenameError(errorText(reason));
    } finally {
      setRenameSubmitting(false);
    }
  };

  return (
    <main className={styles.root} data-creative-asset-library-page>
      <aside className={styles.capabilityBar} role='note' data-asset-upload-limits>
        <p><strong>手动上传：</strong>支持图片和视频，单文件最大 64 MB；暂不支持手动上传音频。</p>
        <Button
          size='mini'
          type='text'
          disabled={library.mutating}
          onClick={() => {
            setRenameError(null);
            setRenameOpen(true);
          }}
        >
          重命名合集
        </Button>
      </aside>

      <CreativeAssetLibrary
        className={styles.library}
        state={library}
        search={search}
        kind={kind}
        scope='library'
        view={view}
        locale={locale}
        selectedIds={EMPTY_SELECTION}
        uploads={uploads.items}
        uploadAccept={CREATIVE_ASSET_MANUAL_UPLOAD_ACCEPT}
        labels={{
          scopeFilter: GLOBAL_SCOPE_ARIA_LABEL,
          searchPlaceholder: '搜索素材标题',
        }}
        onSearchChange={setSearch}
        onKindChange={setKind}
        onScopeChange={(_scope: CreativeAssetScope) => undefined}
        onViewChange={setView}
        onSelectionChange={() => undefined}
        onUploadFiles={handleUploadFiles}
        onCreateText={() => {
          setTextError(null);
          setTextDraft(EMPTY_CREATIVE_TEXT_ASSET_FORM);
          setTextModalOpen(true);
        }}
        onOpenAsset={setPreviewAsset}
        onEditAsset={openEdit}
        onDownloadAsset={downloadAsset}
        onRemoveAsset={(asset) => {
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
        title='删除素材'
        confirmLoading={deleteSubmitting}
        okButtonProps={{ status: 'danger' }}
        okText='删除'
        cancelText='取消'
        maskClosable={!deleteSubmitting}
        closable={!deleteSubmitting}
        getPopupContainer={popupContainer}
        onOk={() => void handleDelete()}
        onCancel={() => {
          if (!deleteSubmitting) setDeletingAsset(null);
        }}
      >
        <div className={styles.modalBody}>
          <p className={styles.deleteText}>确定删除“{deletingAsset?.title}”吗？原始文件也会被永久删除，且无法恢复。</p>
          {deleteError ? <p className={styles.modalError} role='alert'>{deleteError}</p> : null}
        </div>
      </Modal>

      <Modal
        visible={renameOpen}
        title='重命名合集'
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
          <p className={styles.modalDescription}>这会更新所有使用当前合集名称的素材。新名称留空会将这些素材设为未分组。</p>
          <label className={styles.field}>
            <span>当前合集名称</span>
            <Input
              value={renameDraft.from}
              maxLength={240}
              disabled={renameSubmitting}
              onChange={(from) => setRenameDraft((draft) => ({ ...draft, from }))}
            />
          </label>
          <label className={styles.field}>
            <span>新合集名称</span>
            <Input
              value={renameDraft.to}
              maxLength={240}
              placeholder='留空表示取消分组'
              disabled={renameSubmitting}
              onChange={(to) => setRenameDraft((draft) => ({ ...draft, to }))}
            />
          </label>
          {renameError ? <p className={styles.modalError} role='alert'>{renameError}</p> : null}
          <footer className={styles.modalFooter}>
            <Button disabled={renameSubmitting} onClick={() => setRenameOpen(false)}>取消</Button>
            <Button type='primary' htmlType='submit' loading={renameSubmitting}>确认更新</Button>
          </footer>
        </form>
      </Modal>
    </main>
  );
};

export default CreativeAssetLibraryPage;
