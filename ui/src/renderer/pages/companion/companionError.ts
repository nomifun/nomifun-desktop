/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * 把后端的 `AgentErrorCode`(SCREAMING_SNAKE_CASE)映射成桌面伙伴气泡的**可执行**文案 i18n key。
 *
 * 同一套 code 出现在两处:
 *  - 流式 `message.stream` 的 `type:"error"` 事件 → `data.code`(`AgentStreamErrorData`)。
 *  - 发送握手失败抛出的 `BackendHttpError.code`。
 * 未知/缺失 code → 退回通用兜底 `nomi.companion.chatError`(原"走神"文案)。
 */

/** 从流式错误事件的 `data`(unknown)里取出 error code,缺失则空串。 */
export function streamErrorCode(data: unknown): string {
  if (data && typeof data === 'object' && 'code' in data) {
    const c = (data as { code?: unknown }).code;
    if (typeof c === 'string') return c;
  }
  return '';
}

/** error code → 气泡文案 i18n key(默认通用兜底)。 */
export function companionErrorKey(code: string): string {
  switch (code) {
    case 'USER_LLM_PROVIDER_AUTH_FAILED':
    case 'USER_LLM_PROVIDER_PERMISSION_DENIED':
    case 'USER_LLM_PROVIDER_BILLING_REQUIRED':
    case 'USER_LLM_PROVIDER_CONFIG_ERROR':
      return 'nomi.companion.err.auth';
    case 'USER_LLM_PROVIDER_MODEL_NOT_FOUND':
    case 'USER_LLM_PROVIDER_UNSUPPORTED_MODEL':
    case 'USER_LLM_PROVIDER_ENDPOINT_NOT_FOUND':
    case 'USER_LLM_PROVIDER_INVALID_REQUEST':
    case 'USER_LLM_PROVIDER_INVALID_TOOL_SCHEMA':
      return 'nomi.companion.err.model';
    case 'USER_LLM_PROVIDER_NETWORK_ERROR':
      return 'nomi.companion.err.network';
    case 'USER_LLM_PROVIDER_RATE_LIMITED':
      return 'nomi.companion.err.rateLimited';
    case 'USER_LLM_PROVIDER_TIMEOUT':
      return 'nomi.companion.err.timeout';
    case 'USER_LLM_PROVIDER_GATEWAY_ERROR':
    case 'UNKNOWN_UPSTREAM_ERROR':
      return 'nomi.companion.err.gateway';
    case 'USER_LLM_PROVIDER_CONTEXT_TOO_LARGE':
      return 'nomi.companion.err.contextTooLarge';
    case 'USER_LLM_PROVIDER_EMPTY_RESPONSE':
      return 'nomi.companion.err.empty';
    default:
      return 'nomi.companion.chatError';
  }
}
