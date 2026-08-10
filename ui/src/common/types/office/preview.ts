/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ConversationId, PreviewSnapshotId } from '@/common/types/ids';

export type { PreviewSnapshotId };

export type PreviewContentType =
  | 'markdown'
  | 'diff'
  | 'code'
  | 'html'
  | 'pdf'
  | 'ppt'
  | 'word'
  | 'excel'
  | 'image'
  | 'url'
  // 小程序：会话工作区里的单文件自包含 HTML，沙箱 iframe 实时渲染。
  // Mini-app: the conversation's single self-contained HTML artifact.
  // Renderer-only — deliberately absent from the Rust `PreviewContentType`
  // enum because this type never crosses the wire.
  | 'miniapp';

export interface PreviewHistoryTarget {
  contentType: PreviewContentType;
  file_path?: string;
  workspace?: string;
  file_name?: string;
  title?: string;
  language?: string;
  conversation_id?: ConversationId;
}

export interface PreviewSnapshotInfo {
  snapshot_id: PreviewSnapshotId;
  label: string;
  created_at: number;
  size: number;
  content_type: PreviewContentType;
  file_name?: string;
  file_path?: string;
}

export interface PreviewUrlResponse {
  url: string;
  capability?: string;
  error?: string;
}

export interface RemoteImageFetchRequest {
  url: string;
}
