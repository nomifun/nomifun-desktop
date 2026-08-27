/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { forwardRef, useCallback, useImperativeHandle, useState } from 'react';
import { Button, Input, Message, Modal, Popover, Switch } from '@arco-design/web-react';
import { FolderOpen, Info, LinkCloud, Plus, Upload } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import { ipcBridge } from '@/common';
import type { IKnowledgeAddContentResult } from '@/common/adapter/ipcBridge';
import type { KnowledgeBaseId } from '@/common/types/ids';
import { isDesktopShell } from '@renderer/utils/platform';
import KnowledgeUrlEntriesEditor from '../KnowledgeUrlEntriesEditor';
import {
  MAX_KNOWLEDGE_SOURCE_ENTRIES,
  parseKnowledgeUrlDrafts,
  type KnowledgeUrlDraft,
} from '../knowledgeUrlEntries';
import { addKnowledgeContent, knowledgeErrorText, notifySourceFetchResult } from '../useKnowledge';

export interface KnowledgeAddContentControlHandle {
  openDocument: (folderOverride?: string) => void;
}

interface KnowledgeAddContentControlProps {
  knowledgeBaseId: KnowledgeBaseId;
  baseRootPath: string;
  defaultFolderPath: string;
  existingUrlCount: number;
  onAdded: (result: IKnowledgeAddContentResult) => Promise<void>;
}

type AddKnowledgeMenuPanelProps = {
  onNewDocument: () => void;
  onImportFolder: () => void;
  onImportWeb: () => void;
  webDisabled: boolean;
};

const AddKnowledgeMenuPanel: React.FC<AddKnowledgeMenuPanelProps> = ({
  onNewDocument,
  onImportFolder,
  onImportWeb,
  webDisabled,
}) => {
  const { t } = useTranslation();
  const itemClass =
    'knowledge-add-menu-item group flex w-full cursor-pointer items-center gap-10px rounded-10px border-0 bg-transparent px-9px py-8px text-left font-[inherit] transition-colors hover:bg-[var(--color-fill-2)] focus-visible:outline-none focus-visible:bg-[var(--color-fill-2)] disabled:cursor-not-allowed disabled:opacity-45';
  const iconClass =
    'grid size-32px shrink-0 place-items-center rounded-9px bg-[var(--color-fill-2)] text-[var(--color-text-2)] transition-colors group-hover:bg-[rgba(var(--primary-6),0.12)] group-hover:text-primary-6';

  return (
    <div className='knowledge-add-menu w-268px p-5px'>
      <div className='px-9px pb-5px pt-4px'>
        <div className='text-12px font-600 text-[var(--color-text-1)]'>
          {t('knowledge.detail.docs.addKnowledge', { defaultValue: '添加知识' })}
        </div>
        <div className='mt-2px text-10px leading-16px text-[var(--color-text-3)]'>
          {t('knowledge.detail.docs.addKnowledgeHint', { defaultValue: '选择内容进入知识库的方式' })}
        </div>
      </div>
      <button type='button' className={itemClass} onClick={onNewDocument}>
        <span className={iconClass}>
          <Plus theme='outline' size='15' />
        </span>
        <span className='min-w-0'>
          <span className='block text-12px font-600 leading-18px text-[var(--color-text-1)]'>
            {t('knowledge.detail.docs.newFile', { defaultValue: '新建文档' })}
          </span>
          <span className='block text-10px leading-16px text-[var(--color-text-3)]'>
            {t('knowledge.detail.docs.newFileHint', { defaultValue: '创建一个空白 Markdown 文档' })}
          </span>
        </span>
      </button>
      <button type='button' className={itemClass} onClick={onImportFolder}>
        <span className={iconClass}>
          <Upload theme='outline' size='15' />
        </span>
        <span className='min-w-0'>
          <span className='block text-12px font-600 leading-18px text-[var(--color-text-1)]'>
            {t('knowledge.detail.docs.importNotes', { defaultValue: '上传笔记' })}
          </span>
          <span className='block text-10px leading-16px text-[var(--color-text-3)]'>
            {t('knowledge.detail.docs.importNotesHint', { defaultValue: '从本地文件夹复制 Markdown 笔记' })}
          </span>
        </span>
      </button>
      <button type='button' className={itemClass} onClick={onImportWeb} disabled={webDisabled}>
        <span className={iconClass}>
          <LinkCloud theme='outline' size='15' />
        </span>
        <span className='min-w-0'>
          <span className='flex items-center justify-between gap-8px text-12px font-600 leading-18px text-[var(--color-text-1)]'>
            {t('knowledge.detail.docs.importWeb', { defaultValue: '网页抓取' })}
            {webDisabled && (
              <span className='text-9px font-500 text-[var(--color-text-3)]'>
                {t('knowledge.detail.docs.webLimitReached', { defaultValue: '已达上限' })}
              </span>
            )}
          </span>
          <span className='block text-10px leading-16px text-[var(--color-text-3)]'>
            {t('knowledge.detail.docs.importWebHint', { defaultValue: '将网页保存为可刷新的 Markdown 快照' })}
          </span>
        </span>
      </button>
    </div>
  );
};

/**
 * Owns the complete add-content interaction so the already-large detail page
 * only coordinates document-tree refresh and selection after an append.
 */
const KnowledgeAddContentControl = forwardRef<
  KnowledgeAddContentControlHandle,
  KnowledgeAddContentControlProps
>(({ knowledgeBaseId, baseRootPath, defaultFolderPath, existingUrlCount, onAdded }, ref) => {
  const { t } = useTranslation();
  const desktop = isDesktopShell();
  const [menuVisible, setMenuVisible] = useState(false);
  const [newFileVisible, setNewFileVisible] = useState(false);
  const [newFilePath, setNewFilePath] = useState('');
  const [creatingDocument, setCreatingDocument] = useState(false);
  const [folderImportVisible, setFolderImportVisible] = useState(false);
  const [folderImportPath, setFolderImportPath] = useState('');
  const [folderImporting, setFolderImporting] = useState(false);
  const [webImportVisible, setWebImportVisible] = useState(false);
  const [webImportEntries, setWebImportEntries] = useState<KnowledgeUrlDraft[]>([{ url: '', title: '' }]);
  const [webImportRendered, setWebImportRendered] = useState(false);
  const [webImporting, setWebImporting] = useState(false);

  const remainingWebSourceSlots = Math.max(0, MAX_KNOWLEDGE_SOURCE_ENTRIES - existingUrlCount);

  const openDocument = useCallback(
    (folderOverride?: string) => {
      const folder = folderOverride ?? defaultFolderPath;
      setMenuVisible(false);
      setNewFilePath(folder ? `${folder}/` : '');
      setNewFileVisible(true);
    },
    [defaultFolderPath],
  );

  useImperativeHandle(ref, () => ({ openDocument }), [openDocument]);

  const handleCreateDocument = async () => {
    if (creatingDocument) return;
    let path = newFilePath.trim();
    if (!path) return;
    if (!path.toLowerCase().endsWith('.md')) path = `${path}.md`;
    const fileTitle = path.split('/').filter(Boolean).at(-1)?.replace(/\.md$/i, '') || path.replace(/\.md$/i, '');
    setCreatingDocument(true);
    try {
      const result = await addKnowledgeContent(knowledgeBaseId, {
        type: 'document',
        path,
        content: `# ${fileTitle}\n`,
      });
      if (result.type !== 'document') throw new Error('Unexpected knowledge content response');
      setNewFileVisible(false);
      setNewFilePath('');
      await onAdded(result);
      Message.success(t('knowledge.actions.createOk'));
    } catch (error) {
      Message.error(knowledgeErrorText(error));
    } finally {
      setCreatingDocument(false);
    }
  };

  const openFolderImport = () => {
    setMenuVisible(false);
    setFolderImportPath('');
    setFolderImportVisible(true);
  };

  const chooseFolderImportPath = async () => {
    if (!desktop) return;
    try {
      const paths = await ipcBridge.dialog.showOpen.invoke({ properties: ['openDirectory'] });
      if (paths?.[0]) setFolderImportPath(paths[0]);
    } catch (error) {
      Message.error(knowledgeErrorText(error));
    }
  };

  const handleFolderImport = async () => {
    if (folderImporting) return;
    const sourcePath = folderImportPath.trim();
    if (!sourcePath) {
      Message.warning(t('knowledge.detail.docs.importNotesPathRequired', { defaultValue: '请先选择笔记文件夹' }));
      return;
    }
    setFolderImporting(true);
    try {
      const result = await addKnowledgeContent(knowledgeBaseId, {
        type: 'local_folder',
        source_path: sourcePath,
        destination_parent_path: defaultFolderPath || undefined,
      });
      if (result.type !== 'local_folder') throw new Error('Unexpected knowledge content response');
      setFolderImportVisible(false);
      setFolderImportPath('');
      await onAdded(result);
      Message.success(
        t('knowledge.detail.docs.importNotesOk', {
          defaultValue: '已导入 {{count}} 篇笔记到“{{folder}}”',
          count: result.imported,
          folder: result.target_directory,
        }),
      );
      if (result.skipped > 0) {
        Message.info(
          t('knowledge.detail.docs.importNotesSkipped', {
            defaultValue: '另有 {{count}} 个非 Markdown 文件未导入',
            count: result.skipped,
          }),
        );
      }
    } catch (error) {
      Message.error(knowledgeErrorText(error));
    } finally {
      setFolderImporting(false);
    }
  };

  const openWebImport = () => {
    if (remainingWebSourceSlots <= 0) return;
    setMenuVisible(false);
    setWebImportEntries([{ url: '', title: '' }]);
    setWebImportRendered(false);
    setWebImportVisible(true);
  };

  const handleWebImport = async () => {
    if (webImporting) return;
    const parsed = parseKnowledgeUrlDrafts(
      webImportEntries,
      webImportRendered,
      remainingWebSourceSlots,
    );
    if (!parsed.ok) {
      if (parsed.reason === 'empty') {
        Message.warning(t('knowledge.studio.webUrlRequired', { defaultValue: '请至少填写一个网址' }));
      } else if (parsed.reason === 'duplicate') {
        Message.warning(
          t('knowledge.studio.webUrlDuplicate', {
            defaultValue: '网址重复，请只保留一条：{{url}}',
            url: parsed.url,
          }),
        );
      } else if (parsed.reason === 'limit') {
        Message.warning(
          t('knowledge.studio.webUrlLimit', {
            defaultValue: '本知识库还可添加 {{limit}} 个网址',
            limit: parsed.limit,
          }),
        );
      } else {
        Message.warning(
          t('knowledge.studio.webUrlInvalid', {
            defaultValue: '网址格式不正确，需以 http:// 或 https:// 开头：{{url}}',
            url: parsed.url,
          }),
        );
      }
      return;
    }

    setWebImporting(true);
    try {
      const result = await addKnowledgeContent(knowledgeBaseId, {
        type: 'web',
        entries: parsed.entries,
      });
      if (result.type !== 'web') throw new Error('Unexpected knowledge content response');
      setWebImportVisible(false);
      setWebImportEntries([{ url: '', title: '' }]);
      await onAdded(result);
      if (result.added === 0 && result.duplicates > 0) {
        Message.info(
          t('knowledge.detail.docs.importWebAllDuplicate', {
            defaultValue: '这些网址已经在知识库中，无需重复添加',
          }),
        );
      } else {
        notifySourceFetchResult(
          t,
          result,
          t('knowledge.detail.docs.importWebOk', {
            defaultValue: '已添加并抓取 {{count}} 个网页',
            count: result.fetched,
          }),
        );
      }
    } catch (error) {
      Message.error(knowledgeErrorText(error));
    } finally {
      setWebImporting(false);
    }
  };

  return (
    <>
      <Popover
        className='knowledge-add-popover'
        trigger='click'
        position='bl'
        popupVisible={menuVisible}
        onVisibleChange={setMenuVisible}
        unmountOnExit
        content={(
          <AddKnowledgeMenuPanel
            webDisabled={remainingWebSourceSlots <= 0}
            onNewDocument={() => openDocument()}
            onImportFolder={openFolderImport}
            onImportWeb={openWebImport}
          />
        )}
      >
        <span className='inline-flex'>
          <Button
            type='text'
            size='mini'
            shape='circle'
            className='knowledge-doc-icon-button'
            icon={<Plus theme='outline' size='15' />}
            loading={creatingDocument || folderImporting || webImporting}
            aria-label={t('knowledge.detail.docs.addKnowledge', { defaultValue: '添加知识' })}
          />
        </span>
      </Popover>

      <Modal
        title={t('knowledge.newFile')}
        visible={newFileVisible}
        onOk={() => void handleCreateDocument()}
        onCancel={() => {
          if (!creatingDocument) setNewFileVisible(false);
        }}
        confirmLoading={creatingDocument}
        okButtonProps={{ disabled: !newFilePath.trim() }}
        closable={!creatingDocument}
        maskClosable={!creatingDocument}
        autoFocus={false}
      >
        <Input
          placeholder={t('knowledge.newFilePlaceholder')}
          value={newFilePath}
          disabled={creatingDocument}
          onChange={setNewFilePath}
          onPressEnter={() => void handleCreateDocument()}
        />
      </Modal>

      <Modal
        title={t('knowledge.detail.docs.importNotes', { defaultValue: '上传笔记' })}
        visible={folderImportVisible}
        onOk={() => void handleFolderImport()}
        onCancel={() => {
          if (!folderImporting) setFolderImportVisible(false);
        }}
        okText={t('knowledge.detail.docs.importNotesAction', { defaultValue: '导入笔记' })}
        cancelText={t('knowledge.actions.cancel', { defaultValue: '取消' })}
        confirmLoading={folderImporting}
        okButtonProps={{ disabled: !folderImportPath.trim() }}
        closable={!folderImporting}
        maskClosable={!folderImporting}
        autoFocus={false}
        unmountOnExit
        style={{ width: 620, maxWidth: 'calc(100vw - 32px)' }}
      >
        <div className='space-y-14px py-2px'>
          <div className='flex gap-9px rounded-12px bg-[var(--color-fill-1)] px-11px py-9px text-12px leading-19px text-[var(--color-text-2)]'>
            <Info theme='outline' size='14' className='mt-2px shrink-0 text-[var(--color-text-3)]' />
            <span>
              {t('knowledge.detail.docs.importNotesDescription', {
                defaultValue: '选择一个已有目录，系统会保留层级并复制其中的 Markdown 笔记；源文件夹不会被修改。',
              })}
            </span>
          </div>
          <div>
            <label className='mb-6px block text-12px font-600 text-[var(--color-text-1)]'>
              {t('knowledge.detail.docs.importNotesPath', { defaultValue: '笔记文件夹' })}
            </label>
            <div className='flex gap-8px'>
              <Input
                className='knowledge-add-modal-input flex-1'
                value={folderImportPath}
                readOnly={desktop}
                disabled={folderImporting}
                placeholder={t('knowledge.detail.docs.importNotesPlaceholder', {
                  defaultValue: '选择电脑上一个已有目录',
                })}
                onChange={setFolderImportPath}
              />
              {desktop && (
                <Button
                  icon={<FolderOpen theme='outline' size='14' />}
                  disabled={folderImporting}
                  onClick={() => void chooseFolderImportPath()}
                >
                  {t('knowledge.detail.docs.chooseFolder', { defaultValue: '选择文件夹' })}
                </Button>
              )}
            </div>
          </div>
          <div className='text-10px leading-16px text-[var(--color-text-3)]'>
            {t('knowledge.detail.docs.importNotesDestination', {
              defaultValue: '将复制到当前知识库目录：{{path}}',
              path: defaultFolderPath
                ? `${baseRootPath.replace(/[\\/]+$/, '')}/${defaultFolderPath}`
                : baseRootPath,
            })}
          </div>
        </div>
      </Modal>

      <Modal
        title={t('knowledge.detail.docs.importWeb', { defaultValue: '网页抓取' })}
        visible={webImportVisible}
        onOk={() => void handleWebImport()}
        onCancel={() => {
          if (!webImporting) setWebImportVisible(false);
        }}
        okText={t('knowledge.detail.docs.importWebAction', { defaultValue: '开始抓取' })}
        cancelText={t('knowledge.actions.cancel', { defaultValue: '取消' })}
        confirmLoading={webImporting}
        okButtonProps={{
          disabled: remainingWebSourceSlots <= 0 || !webImportEntries.some((entry) => entry.url.trim().length > 0),
        }}
        closable={!webImporting}
        maskClosable={!webImporting}
        autoFocus={false}
        unmountOnExit
        style={{ width: 720, maxWidth: 'calc(100vw - 32px)' }}
      >
        <div className='space-y-14px py-2px'>
          <div className='flex items-start justify-between gap-12px'>
            <div>
              <div className='text-12px font-600 text-[var(--color-text-1)]'>
                {t('knowledge.detail.docs.webUrlList', { defaultValue: '网址列表' })}
              </div>
              <div className='mt-3px text-10px leading-16px text-[var(--color-text-3)]'>
                {t('knowledge.detail.docs.importWebDescription', {
                  defaultValue: '网页会转换为 Markdown 快照并加入文档树，之后可随时刷新。',
                })}
              </div>
            </div>
            <span className='shrink-0 rounded-7px bg-[var(--color-fill-1)] px-7px py-3px text-10px text-[var(--color-text-3)]'>
              {t('knowledge.detail.docs.webSlotsRemaining', {
                defaultValue: '还可添加 {{count}} 条',
                count: remainingWebSourceSlots,
              })}
            </span>
          </div>
          <KnowledgeUrlEntriesEditor
            entries={webImportEntries}
            maxEntries={Math.max(1, remainingWebSourceSlots)}
            disabled={webImporting}
            onChange={setWebImportEntries}
          />
          <div className='flex items-center gap-9px rounded-11px bg-[var(--color-fill-1)] px-10px py-8px'>
            <Switch
              size='small'
              checked={webImportRendered}
              disabled={webImporting}
              onChange={setWebImportRendered}
            />
            <div className='min-w-0'>
              <div className='text-11px font-500 text-[var(--color-text-2)]'>
                {t('knowledge.studio.webBrowserRenderLabel', { defaultValue: '用真实浏览器渲染后抓取' })}
              </div>
              <div className='text-10px leading-15px text-[var(--color-text-3)]'>
                {t('knowledge.studio.webBrowserRenderNote', { defaultValue: '适合 JS 渲染的单页应用' })}
              </div>
            </div>
          </div>
        </div>
      </Modal>
    </>
  );
});

KnowledgeAddContentControl.displayName = 'KnowledgeAddContentControl';

export default KnowledgeAddContentControl;
