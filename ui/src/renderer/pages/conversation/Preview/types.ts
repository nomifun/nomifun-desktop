/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 * Based on AionUi (https://github.com/iOfficeAI/AionUi)
 */

/**
 * Preview 模块类型定义
 * Preview module type definitions
 *
 * 注意：核心类型定义在 @/common/types/office/preview，用于跨进程通信
 * Note: Core type definitions are in @/common/types/office/preview for IPC
 */

// 重新导出 common 中的类型，方便模块内使用
// Re-export types from common for convenience within module
export type {
  PreviewContentType,
  PreviewHistoryTarget,
  PreviewSnapshotInfo,
  RemoteImageFetchRequest,
} from '@/common/types/office/preview';
