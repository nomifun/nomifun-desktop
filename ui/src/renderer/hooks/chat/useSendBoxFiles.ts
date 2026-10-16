import { useCallback } from 'react';
import type { FileMetadata } from '@/renderer/services/FileService';
import type { FileOrFolderItem } from '@/renderer/utils/file/fileTypes';

/**
 * 创建通用的setUploadFile函数
 * 支持函数式更新，避免闭包陷阱
 */
export const createSetUploadFile = <T extends Record<string, unknown>>(
  mutate: (fn: (prev: T | undefined) => T) => void
) => {
  return useCallback(
    (uploadFile: string[] | ((prev: string[]) => string[])) => {
      mutate((prev) => {
        // 取出最新的上传文件列表，保证函数式更新正确 / Derive latest upload list to keep functional updates accurate
        const previousUploadFile = Array.isArray(prev?.uploadFile) ? (prev?.uploadFile as string[]) : [];
        const newUploadFile = typeof uploadFile === 'function' ? uploadFile(previousUploadFile) : uploadFile;
        return { ...prev, uploadFile: newUploadFile } as unknown as T;
      });
    },
    [mutate]
  );
};

interface UseSendBoxFilesProps {
  setAtPath: (atPath: Array<string | FileOrFolderItem>) => void;
  setUploadFile: (uploadFile: string[] | ((prev: string[]) => string[])) => void;
}

/**
 * Manage attachment state in the conversation composer.
 */
export const useSendBoxFiles = ({ setAtPath, setUploadFile }: UseSendBoxFilesProps) => {
  // 处理拖拽或粘贴的文件
  const handleFilesAdded = useCallback(
    (files: FileMetadata[]) => {
      const file_paths = files.map((file) => file.path);
      // 使用函数式更新，基于最新状态而不是闭包中的状态
      setUploadFile((prevUploadFile) => [...prevUploadFile, ...file_paths]);
    },
    [setUploadFile]
  );

  // 清理文件状态
  const clearFiles = useCallback(() => {
    setAtPath([]);
    setUploadFile([]);
  }, [setAtPath, setUploadFile]);

  return {
    handleFilesAdded,
    clearFiles,
  };
};
