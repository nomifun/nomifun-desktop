/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useCallback, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Message } from '@arco-design/web-react';
import type { FileMetadata } from '@renderer/services/FileService';
import { FileService } from '@renderer/services/FileService';
import type { UploadSource } from '@renderer/hooks/file/useUploadState';
import type { ConversationId } from '@/common/types/ids';

export interface UseDragUploadOptions {
  onFilesAdded?: (files: FileMetadata[]) => void;
  /** Conversation ID for WebUI file uploads */
  conversation_id?: ConversationId;
  /** Upload surface used for progress scoping (defaults to 'sendbox') */
  source?: UploadSource;
}

export const useDragUpload = ({ onFilesAdded, conversation_id, source = 'sendbox' }: UseDragUploadOptions) => {
  const { t } = useTranslation();
  const [isFileDragging, setIsFileDragging] = useState(false);

  // 拖拽计数器，防止状态闪烁
  const dragCounter = useRef(0);

  const handleDragOver = useCallback(
    (e: React.DragEvent) => {
      e.preventDefault();
      e.stopPropagation();

      if (!isFileDragging) {
        setIsFileDragging(true);
        dragCounter.current += 1;
      }
    },
    [isFileDragging]
  );

  const handleDragEnter = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();

    dragCounter.current += 1;
    setIsFileDragging(true);
  }, []);

  const handleDragLeave = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();

    dragCounter.current -= 1;

    if (dragCounter.current <= 0) {
      dragCounter.current = 0;
      setIsFileDragging(false);
    }
  }, []);

  const handleDrop = useCallback(
    async (e: React.DragEvent) => {
      e.preventDefault();
      e.stopPropagation();

      // 重置状态
      dragCounter.current = 0;
      setIsFileDragging(false);

      if (!onFilesAdded) return;

      try {
        const droppedFiles = e.nativeEvent.dataTransfer!.files;

        if (droppedFiles.length > 0) {
          const processedFiles = await FileService.processDroppedFiles(droppedFiles, conversation_id, source);

          if (processedFiles.length > 0) {
            onFilesAdded(processedFiles);
          }
        }
      } catch (err) {
        console.error('Failed to process dropped files:', err);
        Message.error(t('conversation.workspace.dragFailed', 'Failed to process dropped files'));
      }
    },
    [conversation_id, onFilesAdded, source, t]
  );

  const dragHandlers = {
    onDragOver: handleDragOver,
    onDragEnter: handleDragEnter,
    onDragLeave: handleDragLeave,
    onDrop: handleDrop,
  };

  return {
    isFileDragging,
    dragHandlers,
  };
};
