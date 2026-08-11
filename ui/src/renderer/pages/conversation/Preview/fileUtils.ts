/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { PreviewContentType } from '@/common/types/office/preview';
import { MINI_APP_FILE_NAME } from '@renderer/pages/miniApps/contract';

/**
 * 文件扩展名到内容类型的映射配置
 * Mapping configuration from file extensions to content types
 */
export const FILE_EXTENSION_MAP: Record<PreviewContentType, readonly string[]> = {
  markdown: ['md', 'markdown', 'mdown', 'mkd'],
  html: ['html', 'htm'],
  pdf: ['pdf'],
  word: ['doc', 'docx', 'odt'],
  ppt: ['ppt', 'pptx', 'odp'],
  excel: ['xls', 'xlsx', 'ods', 'csv'],
  image: ['png', 'jpg', 'jpeg', 'gif', 'svg', 'webp', 'bmp', 'ico', 'tif', 'tiff', 'avif'],
  code: [], // code 作为默认类型，不需要显式映射 / code is the default type, no explicit mapping needed
  diff: ['diff', 'patch'],
  url: [], // url 类型用于网页预览，无扩展名映射 / url type for web preview, no extension mapping
  // 小程序按精确文件名识别（见下方 getContentTypeByExtension），不走扩展名映射。
  // Mini-apps are matched by exact basename below, not by extension.
  miniapp: [],
};

/**
 * 从文件路径中提取文件扩展名
 * Extract file extension from file path
 *
 * @param file_path - 文件路径 / File path
 * @returns 文件扩展名（小写），如果没有扩展名则返回空字符串 / File extension in lowercase, or empty string if no extension
 *
 * @example
 * ```ts
 * getFileExtension('document.pdf') // => 'pdf'
 * getFileExtension('archive.tar.gz') // => 'gz'
 * getFileExtension('noextension') // => ''
 * getFileExtension('image.PNG') // => 'png'
 * ```
 */
export const getFileExtension = (file_path: string): string => {
  if (!file_path) return '';

  const lastDotIndex = file_path.lastIndexOf('.');
  // 没有点号，或点号在最后（如 "file."），返回空字符串
  // No dot, or dot at the end (e.g., "file."), return empty string
  if (lastDotIndex === -1 || lastDotIndex === file_path.length - 1) {
    return '';
  }

  return file_path.substring(lastDotIndex + 1).toLowerCase();
};

/**
 * 从文件路径中提取文件名（basename）
 * Extract the basename from a file path (handles both separators)
 */
export const getFileBaseName = (file_path: string): string => file_path.split(/[\\/]/).pop() ?? file_path;

/**
 * 根据文件扩展名确定预览内容类型
 * Determine preview content type based on file extension
 *
 * 例外：工作区根目录下的 `miniapp.html` 是小程序契约的唯一产物，必须解析为
 * `miniapp` 而不是 `html` —— 否则文件树打开它会得到第二个 tab。
 * One exception: `miniapp.html` is the mini-app contract's single artifact and
 * classifies as `miniapp`, never `html`. Both openers (the workspace file tree
 * and `useAutoPreviewMiniApp`) must land on ONE tab; classifying by extension
 * here would give the tree an `html` tab beside the auto-opened `miniapp` one.
 *
 * @param file_path - 文件路径或文件名 / File path or file name
 * @returns 预览内容类型 / Preview content type
 *
 * @example
 * ```ts
 * getContentTypeByExtension('README.md') // => 'markdown'
 * getContentTypeByExtension('index.html') // => 'html'
 * getContentTypeByExtension('report.pdf') // => 'pdf'
 * getContentTypeByExtension('script.ts') // => 'code'
 * getContentTypeByExtension('image.png') // => 'image'
 * getContentTypeByExtension('/ws/miniapp.html') // => 'miniapp'
 * ```
 */
export const getContentTypeByExtension = (file_path: string): PreviewContentType => {
  if (getFileBaseName(file_path).toLowerCase() === MINI_APP_FILE_NAME) return 'miniapp';

  const ext = getFileExtension(file_path);
  if (!ext) return 'code'; // 没有扩展名，默认为 code / No extension, default to code

  // 遍历映射表查找匹配的内容类型 / Iterate through mapping to find matching content type
  for (const [contentType, extensions] of Object.entries(FILE_EXTENSION_MAP)) {
    if (extensions.includes(ext)) {
      return contentType as PreviewContentType;
    }
  }

  // 未找到匹配的扩展名，默认为 code / No matching extension found, default to code
  return 'code';
};

/**
 * 检查文件是否为图片类型
 * Check if file is an image type
 *
 * @param file_path - 文件路径 / File path
 * @returns 是否为图片 / Whether it's an image
 */
export const isImageFile = (file_path: string): boolean => {
  return getContentTypeByExtension(file_path) === 'image';
};

/**
 * 检查文件是否为文本类型（可编辑）
 * Check if file is a text type (editable)
 *
 * @param file_path - 文件路径 / File path
 * @returns 是否为文本类型 / Whether it's a text type
 */
export const isTextFile = (file_path: string): boolean => {
  const contentType = getContentTypeByExtension(file_path);
  // `miniapp` is an HTML document too — it just renders in a sandboxed frame.
  return ['markdown', 'html', 'code', 'miniapp'].includes(contentType);
};
