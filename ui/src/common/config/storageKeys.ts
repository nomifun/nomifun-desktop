/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 * Based on AionUi (https://github.com/iOfficeAI/AionUi)
 */

/**
 * Centralized localStorage keys for the application
 * 应用程序的集中式 localStorage 键管理
 *
 * All localStorage keys should be defined here to:
 * - Avoid key conflicts
 * - Make it easy to find and manage all persisted states
 * - Provide a single source of truth for storage key names
 */
export const STORAGE_KEYS = {
  /** Workspace tree collapse state / 工作空间目录树折叠状态 */
  WORKSPACE_TREE_COLLAPSE: 'nomifun_workspace_collapse_state',

  /** Sidebar collapse state / 侧边栏折叠状态 */
  SIDEBAR_COLLAPSE: 'nomifun_sider_collapsed',

  /** Theme preference / 主题偏好 */
  THEME: 'nomifun_theme',

  /** Language preference / 语言偏好 */
  LANGUAGE: 'nomifun_language',
} as const;
