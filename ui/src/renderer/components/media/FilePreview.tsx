/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Close, FolderOpen } from '@icon-park/react';
import React, { useEffect, useState } from 'react';
import { getFileExtension } from '@/renderer/services/FileService';
import { ipcBridge } from '@/common';
import { Image, Message } from '@arco-design/web-react';
import fileIcon from '@/renderer/assets/icons/file-icon.svg';
import ComposerAttachmentTile from '@/renderer/components/chat/ComposerAttachmentTile';
import attachmentStyles from '@/renderer/components/chat/ComposerAttachments.module.css';
import ImageLightbox from './ImageLightbox';
import { downloadFileFromPath } from '@/renderer/utils/file/download';
import { usePreviewLauncher } from '@/renderer/hooks/file/usePreviewLauncher';
import { getFileTypeInfo } from '@/renderer/utils/file/fileType';
import { isPreviewSupportedExt } from '@/renderer/pages/conversation/Workspace/utils/filePreview';

const IMAGE_EXTS = new Set(['.jpg', '.jpeg', '.png', '.gif', '.bmp', '.webp', '.svg']);

// Substring from the base64-encoded "Image not found" placeholder SVG returned by getImageBase64 on ENOENT.
// This is the aligned base64 encoding of ">Image not found<" within the SVG data URL.
const IMAGE_NOT_FOUND_B64_MARKER = 'kltYWdlIG5vdCBmb3VuZD';
const MAX_IMAGE_RETRIES = 5;
const IMAGE_RETRY_DELAY_MS = 800;

const isImageFile = (path: string): boolean => {
  const ext = path.toLowerCase().slice(path.lastIndexOf('.'));
  return IMAGE_EXTS.has(ext);
};

// 格式化文件大小
const formatFileSize = (bytes: number): string => {
  if (bytes === 0) return '0B';
  if (bytes < 1024) return `${bytes}B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)}KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)}MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)}GB`;
};

interface FilePreviewProps {
  path: string;
  /** Omit for pure read-only display (no delete affordance is rendered). */
  onRemove?: () => void;
  readonly?: boolean;
  compact?: boolean;
  ordinal?: number;
  isDirectory?: boolean;
  inactive?: boolean;
}

const FilePreview: React.FC<FilePreviewProps> = ({ path, onRemove, readonly = false, compact = false, ordinal, isDirectory = false, inactive = false }) => {
  // Defensive check: ensure path is a string
  if (typeof path !== 'string') {
    console.error('[FilePreview] Invalid path type:', typeof path, path);
    return null;
  }

  const isImage = !isDirectory && isImageFile(path);
  // 直接从路径中提取文件名，不清理时间戳后缀
  // Extract filename directly from path without cleaning timestamp suffix
  const file_name = path.split(/[\\/]/).pop() || '';
  const fileExt = getFileExtension(path).toUpperCase().replace('.', '');
  const [imageUrl, setImageUrl] = useState<string>('');
  const [fileSize, setFileSize] = useState<string>('');
  const [viewing, setViewing] = useState(false);
  const { launchPreview, canPreview, loading: opening } = usePreviewLauncher();

  useEffect(() => {
    // 获取文件大小
    ipcBridge.fs.getFileMetadata
      .invoke({ path })
      .then((metadata) => {
        setFileSize(formatFileSize(metadata.size));
      })
      .catch((error) => {
        console.error('[FilePreview] Failed to get file metadata:', { path, error });
      });

    // 如果是图片，获取图片的base64
    // Retry when the file is not found yet (race condition: display message rendered
    // before the backend finishes copying the pasted image to the workspace).
    if (isImage) {
      let cancelled = false;
      let retryCount = 0;
      let retryTimer: ReturnType<typeof setTimeout>;

      const loadImage = () => {
        ipcBridge.fs.getImageBase64
          .invoke({ path })
          .then((base64) => {
            if (cancelled) return;
            if (!base64) {
              setImageUrl('');
              return;
            }
            if (base64.includes(IMAGE_NOT_FOUND_B64_MARKER) && retryCount < MAX_IMAGE_RETRIES) {
              retryCount++;
              retryTimer = setTimeout(loadImage, IMAGE_RETRY_DELAY_MS);
            } else {
              setImageUrl(base64);
            }
          })
          .catch((error) => {
            if (cancelled) return;
            console.error('[FilePreview] Failed to load image:', { path, error });
          });
      };

      loadImage();

      return () => {
        cancelled = true;
        clearTimeout(retryTimer);
      };
    }

    return undefined;
  }, [isImage, path]);

  const handleRemove = (e: React.MouseEvent) => {
    e.stopPropagation();
    onRemove?.();
  };

  const openFile = async () => {
    if (isImage) { setViewing(true); return; }
    try {
      if (!isDirectory && canPreview && isPreviewSupportedExt(file_name)) {
        const { contentType, language } = getFileTypeInfo(file_name);
        await launchPreview({ originalPath: path, file_name, contentType, language, editable: false });
      } else await ipcBridge.shell.openFile.invoke(path);
    } catch (error) { Message.error(error instanceof Error ? error.message : String(error)); }
  };

  if (compact) return <>
    <ComposerAttachmentTile title={file_name} detail={[path, isDirectory ? '文件夹' : `${fileExt} ${fileSize}`, inactive ? '本轮不使用' : ''].filter(Boolean).join('\n')} ordinal={ordinal} inactive={inactive} onRemove={!readonly ? onRemove : undefined}>
      <button type='button' className={attachmentStyles.open} aria-label={`${isImage ? '查看图片' : '查看文件'}：${file_name}`} disabled={opening || (isImage && !imageUrl)} onClick={() => void openFile()}>
        {isDirectory ? <FolderOpen size={24} fill='currentColor' /> : isImage && imageUrl ? <img src={imageUrl} alt={file_name} /> : <img className={attachmentStyles.fileIcon} src={fileIcon} alt={isImage ? '正在读取图片' : fileExt} />}
      </button>
    </ComposerAttachmentTile>
    {viewing && imageUrl && <ImageLightbox key={imageUrl} src={imageUrl} title={file_name} onClose={() => setViewing(false)} onDownload={() => downloadFileFromPath(path, file_name)} />}
  </>;

  if (isImage) {
    return (
      <div className='relative inline-block'>
        {/* border-1px 是宽度（`border-1` 只是 --bg-1 颜色），border-arco-2 才是存在的
            边框色（`b-color-border-2` 查一个叫 color-border-2 的颜色，产出 0 条 CSS）。
            Before this, the thumbnail frame was a 3px `medium` border in --bg-1. */}
        <div className='rd-8px overflow-hidden border-1px border-solid border-arco-2'>
          <Image
            src={imageUrl || undefined}
            alt={file_name}
            width={60}
            height={60}
            className='object-cover cursor-pointer'
            style={{ display: imageUrl ? 'block' : 'none' }}
            preview={Boolean(imageUrl)}
          />
          {!imageUrl && <div className='w-60px h-60px bg-3'></div>}
        </div>
        {!readonly && onRemove && (
          <div
            className='absolute -top-4px -right-4px w-16px h-16px rd-50% bg-white dark:bg-gray-700 cursor-pointer flex items-center justify-center shadow-md hover:shadow-lg transition-all z-10 border-1px border-solid border-gray-200 dark:border-gray-600'
            onClick={handleRemove}
          >
            <Close theme='filled' size='10' fill='#666' />
          </div>
        )}
      </div>
    );
  }

  return (
    <div className='relative inline-block mb-10px'>
      <div
        className='h-60px flex items-center gap-12px px-12px rd-8px bg-2 border border-solid'
        style={{ borderColor: 'var(--border-base)', boxShadow: '0 0 0 1px rgba(0,0,0,0.02)' }}
      >
        <div className='w-40px h-40px rd-8px flex items-center justify-center flex-shrink-0'>
          <img className='w-full h-full object-contain' src={fileIcon} alt='File Icon' />
        </div>
        <div className='flex flex-col gap-2px min-w-0'>
          <span className='text-14px text-t-primary max-w-150px truncate'>{file_name}</span>
          <span className='text-12px text-t-secondary'>
            {fileExt}: {fileSize || '...'}
          </span>
        </div>
      </div>
      {!readonly && onRemove && (
        <div
          className='absolute -top-4px -right-4px w-16px h-16px rd-50% bg-white dark:bg-gray-700 cursor-pointer flex items-center justify-center shadow-md hover:shadow-lg transition-all z-10 border-1px border-solid border-gray-200 dark:border-gray-600'
          onClick={handleRemove}
        >
          <Close theme='filled' size='10' fill='#666' />
        </div>
      )}
    </div>
  );
};

export default FilePreview;
