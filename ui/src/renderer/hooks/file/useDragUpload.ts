/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useCallback, useEffect, useRef, useState, type DragEvent, type RefObject } from 'react';
import { useTranslation } from 'react-i18next';
import { Message } from '@arco-design/web-react';
import { isTauriRuntime } from '@/common/adapter/tauriRuntime';
import { FileService, UPLOAD_ABORTED_ERROR, type FileMetadata } from '@renderer/services/FileService';
import { trackUpload, type UploadSource } from '@renderer/hooks/file/useUploadState';
import type { ConversationId } from '@/common/types/ids';
import { useNativeFileDrop } from './useNativeFileDrop';

export interface UseDragUploadOptions {
  /** Providing a target enables scoped native desktop drops. */
  targetRef?: RefObject<HTMLElement | null>;
  onFilesAdded?: (files: FileMetadata[]) => void;
  /** Conversation ID for WebUI file uploads */
  conversation_id?: ConversationId;
  /** Upload surface used for progress scoping (defaults to 'sendbox') */
  source?: UploadSource;
}

// Text/URL drags must retain the editor's normal insertion behavior.
const hasFiles = (transfer: DataTransfer | null): boolean => Boolean(transfer && (
  Array.from(transfer.types ?? []).includes('Files') || transfer.files?.length ||
  Array.from(transfer.items ?? []).some(item => item.kind === 'file')
));

// React portals bubble events through the composer even when their DOM is elsewhere.
const isOnTarget = (event: DragEvent): boolean => event.target instanceof Node && event.currentTarget.contains(event.target);

export const useDragUpload = ({ targetRef, onFilesAdded, conversation_id, source = 'sendbox' }: UseDragUploadOptions) => {
  const { t } = useTranslation();
  const [isFileDragging, setIsFileDragging] = useState(false);
  const dragCounter = useRef(0);
  const lifecycle = useRef(0);
  const unusedTargetRef = useRef<HTMLElement>(null);

  const resetDragState = useCallback(() => {
    dragCounter.current = 0;
    setIsFileDragging(false);
  }, []);

  useEffect(() => {
    resetDragState();
    return () => { lifecycle.current++; };
  }, [conversation_id, source, resetDragState]);

  const processFiles = useCallback(async (process: () => Promise<FileMetadata[]>, warnIfEmpty = false) => {
    if (!onFilesAdded) return;
    const startedAt = lifecycle.current;
    try {
      const files = await process();
      // A completed drop belongs to the composer that started it.
      if (startedAt !== lifecycle.current) return;
      if (files.length) onFilesAdded(files);
      else if (warnIfEmpty) Message.warning(t('conversation.sendbox.dropNoFiles'));
    } catch (error) {
      if (startedAt !== lifecycle.current) return;
      if (error instanceof Error && error.message === UPLOAD_ABORTED_ERROR) return;
      console.error('Failed to process dropped files:', error);
      Message.error(t('conversation.workspace.dragFailed', 'Failed to process dropped files'));
    }
  }, [onFilesAdded, t]);

  useNativeFileDrop({
    targetRef: targetRef ?? unusedTargetRef,
    enabled: Boolean(targetRef && onFilesAdded),
    onHoverChange: setIsFileDragging,
    onDrop: paths => {
      if (!onFilesAdded) return;
      resetDragState();
      // Metadata inspection is asynchronous too. Reuse the upload guard so an
      // immediate Enter cannot send before these attachments are ready.
      let cancelled = false;
      const tracker = trackUpload(0, {
        source,
        name: paths.map(path => path.split(/[\\/]/).pop() || path).join(', '),
        conversationId: conversation_id,
        onAbort: () => { cancelled = true; },
      });
      void processFiles(async () => {
        try {
          const files = await FileService.processNativePaths(paths);
          if (cancelled) throw new Error(UPLOAD_ABORTED_ERROR);
          return files;
        } finally {
          tracker.finish();
        }
      }, true);
    },
  });

  const handleDragEnter = useCallback((event: DragEvent) => {
    if ((targetRef && isTauriRuntime()) || !onFilesAdded || !isOnTarget(event) || !hasFiles(event.dataTransfer)) return;
    event.preventDefault();
    event.stopPropagation();
    dragCounter.current++;
    setIsFileDragging(true);
  }, [onFilesAdded, targetRef]);

  const handleDragOver = useCallback((event: DragEvent) => {
    if ((targetRef && isTauriRuntime()) || !onFilesAdded || !isOnTarget(event) || !hasFiles(event.dataTransfer)) return;
    event.preventDefault();
    event.stopPropagation();
    event.dataTransfer.dropEffect = 'copy';
    setIsFileDragging(true);
  }, [onFilesAdded, targetRef]);

  const handleDragLeave = useCallback((event: DragEvent) => {
    if ((targetRef && isTauriRuntime()) || !onFilesAdded || !isOnTarget(event) || !hasFiles(event.dataTransfer)) return;
    event.preventDefault();
    event.stopPropagation();
    dragCounter.current = Math.max(0, dragCounter.current - 1);
    if (!dragCounter.current) setIsFileDragging(false);
  }, [onFilesAdded, targetRef]);

  const handleDrop = useCallback((event: DragEvent) => {
    if (!onFilesAdded || !isOnTarget(event) || !hasFiles(event.dataTransfer)) return;
    event.preventDefault();
    event.stopPropagation();
    resetDragState();
    // Tauri delivers host paths via the native listener. Never add a second
    // HTTP-uploaded copy if the webview also emits a DOM file drop.
    if (targetRef && isTauriRuntime()) return;
    const transfer = event.dataTransfer;
    const fileItems = Array.from(transfer.items ?? []).filter(item => item.kind === 'file');
    const files = Array.from(transfer.files).filter((_, index) =>
      !fileItems[index]?.webkitGetAsEntry?.()?.isDirectory
    );
    if (!files.length) {
      Message.warning(t('conversation.sendbox.dropNoFiles'));
      return;
    }
    const fileList = Object.assign(files, { item: (index: number) => files[index] ?? null }) as unknown as FileList;
    void processFiles(() => FileService.processDroppedFiles(fileList, conversation_id, source));
  }, [conversation_id, onFilesAdded, processFiles, resetDragState, source, t, targetRef]);

  return {
    isFileDragging,
    dragHandlers: { onDragOver: handleDragOver, onDragEnter: handleDragEnter,
      onDragLeave: handleDragLeave, onDrop: handleDrop },
  };
};
