/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type { IWorkspaceFlatFile } from '@/common/adapter/ipcBridge';
import type { TChatConversation } from '@/common/config/storage';
import type { ConversationId } from '@/common/types/ids';
import { parseError, uuidv7 } from '@/common/utils';
import { formatTimestamp, joinFilePath, sanitizeFileName } from '@/renderer/utils/chat/conversationExport';
import { EXPORT_IO_TIMEOUT_MS, fetchConversationExportMessages, withTimeout } from '@/renderer/utils/chat/conversationExportData';
import { isDesktopShell } from '@/renderer/utils/platform';
import { Message } from '@arco-design/web-react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { ExportTask, ExportZipFile } from '../types';
import {
  appendWorkspaceFilesToZip,
  buildConversationJson,
  buildConversationMarkdown,
  buildTopicFolderName,
} from '../utils/exportHelpers';

type UseExportParams = {
  conversations: TChatConversation[];
  selectedConversationIds: Set<ConversationId>;
  setSelectedConversationIds: React.Dispatch<React.SetStateAction<Set<ConversationId>>>;
  onBatchModeChange?: (value: boolean) => void;
};

type ExportRequest = { id: string; canceled: boolean; zipStarted: boolean };
class ExportCanceledError extends Error {}

export const useExport = ({
  conversations,
  selectedConversationIds,
  setSelectedConversationIds,
  onBatchModeChange,
}: UseExportParams) => {
  const [exportTask, setExportTask] = useState<ExportTask>(null);
  const [exportModalVisible, setExportModalVisible] = useState(false);
  const [exportTargetPath, setExportTargetPath] = useState('');
  const [exportModalLoading, setExportModalLoading] = useState(false);
  const [showExportDirectorySelector, setShowExportDirectorySelector] = useState(false);
  const activeRequestRef = useRef<ExportRequest | null>(null);
  const modalRevisionRef = useRef(0);
  const directoryRevisionRef = useRef(0);
  const { t } = useTranslation();

  const cancelZip = useCallback((request: ExportRequest) => {
    if (!request.zipStarted) return;
    void ipcBridge.fs.cancelZip.invoke({ request_id: request.id }).catch((error) => {
      console.warn('Failed to cancel conversation export:', error);
    });
  }, []);

  const cancelActiveExport = useCallback(() => {
    const request = activeRequestRef.current;
    if (request) {
      request.canceled = true;
      activeRequestRef.current = null;
      cancelZip(request);
    }
  }, [cancelZip]);

  useEffect(() => () => {
    modalRevisionRef.current += 1;
    cancelActiveExport();
  }, [cancelActiveExport]);

  const getDesktopPath = useCallback(async (): Promise<string> => {
    try {
      return (await ipcBridge.application.getPath.invoke({ name: 'desktop' })) || '';
    } catch {
      return '';
    }
  }, []);

  const closeExportModal = useCallback(() => {
    modalRevisionRef.current += 1;
    cancelActiveExport();
    setExportModalVisible(false);
    setShowExportDirectorySelector(false);
    setExportTask(null);
    setExportTargetPath('');
    setExportModalLoading(false);
  }, [cancelActiveExport]);

  const openExportModal = useCallback(async (task: NonNullable<ExportTask>) => {
    cancelActiveExport();
    const revision = ++modalRevisionRef.current;
    const directoryRevision = directoryRevisionRef.current;
    setExportTask(task);
    setExportTargetPath('');
    setExportModalLoading(false);
    setExportModalVisible(true);
    const desktopPath = await getDesktopPath();
    if (modalRevisionRef.current === revision && directoryRevisionRef.current === directoryRevision) {
      setExportTargetPath(desktopPath);
    }
  }, [cancelActiveExport, getDesktopPath]);

  const handleSelectExportDirectoryFromModal = useCallback((paths: string[] | undefined) => {
    setShowExportDirectorySelector(false);
    if (paths?.length && !activeRequestRef.current) {
      directoryRevisionRef.current += 1;
      setExportTargetPath(paths[0]);
    }
  }, []);

  const handleSelectExportFolder = useCallback(async () => {
    if (activeRequestRef.current) return;
    if (!isDesktopShell()) {
      setShowExportDirectorySelector(true);
      return;
    }
    const revision = modalRevisionRef.current;
    try {
      const defaultPath = exportTargetPath || (await getDesktopPath());
      if (revision !== modalRevisionRef.current) return;
      const folders = await ipcBridge.dialog.showOpen.invoke({
        properties: ['openDirectory', 'createDirectory'],
        defaultPath: defaultPath || undefined,
      });
      if (revision === modalRevisionRef.current && folders?.length && !activeRequestRef.current) {
        directoryRevisionRef.current += 1;
        setExportTargetPath(folders[0]);
      }
    } catch (error) {
      if (revision !== modalRevisionRef.current) return;
      console.error('Failed to open export directory dialog:', error);
      Message.error(`${t('conversation.history.exportFailed')}: ${parseError(error)}`);
    }
  }, [exportTargetPath, getDesktopPath, t]);

  const fetchConversationWorkspaceFiles = useCallback(async (
    conversation: TChatConversation,
    assertActive: () => void
  ): Promise<IWorkspaceFlatFile[]> => {
    const workspace = conversation.extra?.workspace?.trim();
    if (!workspace) return [];
    // Materialize a managed Session workspace through its canonical owner before
    // asking the file service for its complete recursive project inventory.
    await withTimeout(
      ipcBridge.conversation.getWorkspace.invoke({ conversation_id: conversation.id, workspace, path: workspace }),
      EXPORT_IO_TIMEOUT_MS,
      `getWorkspace:${conversation.id}`
    );
    assertActive();
    const files = await withTimeout(
      ipcBridge.fs.listWorkspaceFiles.invoke({ root: workspace, workspace }),
      EXPORT_IO_TIMEOUT_MS,
      `listWorkspaceFiles:${conversation.id}`
    );
    assertActive();
    return files;
  }, []);

  const buildConversationExportFiles = useCallback(async (
    conversation: TChatConversation,
    assertActive: () => void
  ): Promise<ExportZipFile[]> => {
    const [messages, workspaceFiles] = await Promise.all([
      fetchConversationExportMessages(conversation.id, assertActive),
      fetchConversationWorkspaceFiles(conversation, assertActive),
    ]);
    assertActive();
    const topicFolderName = buildTopicFolderName(conversation);
    const files: ExportZipFile[] = [
      { name: `${topicFolderName}/conversation/conversation.json`, content: buildConversationJson(conversation, messages) },
      { name: `${topicFolderName}/conversation/conversation.md`, content: buildConversationMarkdown(conversation, messages) },
    ];
    appendWorkspaceFilesToZip(files, workspaceFiles, topicFolderName);
    return files;
  }, [fetchConversationWorkspaceFiles]);

  const handleExportConversation = useCallback((conversation: TChatConversation) => {
    void openExportModal({ mode: 'single', conversation });
  }, [openExportModal]);

  const handleBatchExport = useCallback(() => {
    if (selectedConversationIds.size === 0) {
      Message.warning(t('conversation.history.batchNoSelection'));
      return;
    }
    void openExportModal({ mode: 'batch', conversation_ids: Array.from(selectedConversationIds) });
  }, [openExportModal, selectedConversationIds, t]);

  const handleConfirmExport = useCallback(async () => {
    if (!exportTask || activeRequestRef.current) return;
    const directory = exportTargetPath.trim();
    if (!directory) {
      Message.warning(t('conversation.history.exportSelectFolder'));
      return;
    }
    const request: ExportRequest = { id: uuidv7(), canceled: false, zipStarted: false };
    activeRequestRef.current = request;
    setExportModalLoading(true);
    const assertActive = () => {
      if (request.canceled || activeRequestRef.current !== request) throw new ExportCanceledError();
    };
    try {
      const selectedConversations = exportTask.mode === 'single'
        ? [exportTask.conversation]
        : exportTask.conversation_ids.map((id) => {
          const conversation = conversations.find((item) => item.id === id);
          if (!conversation) throw new Error(`Selected conversation is unavailable: ${id}`);
          return conversation;
        });
      if (!selectedConversations.length) {
        Message.warning(t('conversation.history.batchNoSelection'));
        return;
      }
      const files = (await Promise.all(selectedConversations.map((conversation) =>
        buildConversationExportFiles(conversation, assertActive)
      ))).flat();
      assertActive();
      const name = exportTask.mode === 'single'
        ? sanitizeFileName(selectedConversations[0].name || selectedConversations[0].id).slice(0, 40)
        : 'batch-export';
      const exportPath = joinFilePath(directory, `${name}-${formatTimestamp()}-${request.id.slice(-8)}.zip`);
      const workspaces = [...new Set(selectedConversations.map((conversation) => conversation.extra?.workspace?.trim()).filter((workspace): workspace is string => Boolean(workspace)))];
      request.zipStarted = true;
      const success = await withTimeout(
        ipcBridge.fs.createZip.invoke({ path: exportPath, files, request_id: request.id, workspaces }),
        EXPORT_IO_TIMEOUT_MS * 8,
        `createZip:${request.id}`
      );
      assertActive();
      if (!success) throw new Error('ZIP creation did not complete');
      Message.success(t('conversation.history.exportSuccess'));
      if (exportTask.mode === 'batch') {
        setSelectedConversationIds(new Set());
        onBatchModeChange?.(false);
      }
      setExportModalVisible(false);
      setExportTask(null);
      setExportTargetPath('');
    } catch (error) {
      if (request.canceled || error instanceof ExportCanceledError) return;
      cancelZip(request);
      console.error('Failed to export conversations:', error);
      Message.error(`${t('conversation.history.exportFailed')}: ${parseError(error)}`);
    } finally {
      if (activeRequestRef.current === request) {
        activeRequestRef.current = null;
        setExportModalLoading(false);
      }
    }
  }, [buildConversationExportFiles, cancelZip, conversations, exportTargetPath, exportTask, onBatchModeChange, setSelectedConversationIds, t]);

  return {
    exportTask,
    exportModalVisible,
    exportTargetPath,
    exportModalLoading,
    showExportDirectorySelector,
    setShowExportDirectorySelector,
    closeExportModal,
    handleSelectExportDirectoryFromModal,
    handleSelectExportFolder,
    handleExportConversation,
    handleBatchExport,
    handleConfirmExport,
  };
};
