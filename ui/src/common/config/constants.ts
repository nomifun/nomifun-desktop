/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Nomi应用程序共用常量
 */

// ===== 文件处理相关常量 =====

/** 用于匹配和清理时间戳后缀的正则表达式 */
export const NOMIFUN_TIMESTAMP_REGEX = /_nomifun_\d{13}(\.\w+)?$/;
export const NOMIFUN_FILES_MARKER = '[[NOMI_FILES]]';

// ===== WebUI 相关常量 =====

/** WebUI default port: 25808 for production, 25809 for development, 25810 for multi-instance dev */
export const WEBUI_DEFAULT_PORT = (() => {
  if (process.env.NODE_ENV === 'production') return 25808;
  if (process.env.NOMIFUN_MULTI_INSTANCE === '1') return 25810;
  return 25809;
})();

// ===== AI Provider 相关常量 =====

/** Stable bare UUIDv7 business ID of the built-in Nomi agent. */
export const NOMI_AGENT_ID = '0190f5fe-7c00-7a00-8000-000000000114';
