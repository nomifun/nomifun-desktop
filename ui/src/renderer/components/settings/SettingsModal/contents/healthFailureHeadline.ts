/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ProviderHealthCheckErrorKind } from '@/common/protocolBindings/ProviderHealthCheckErrorKind';
import type { ProviderHealthCheckResponse } from '@/common/protocolBindings/ProviderHealthCheckResponse';

type Translate = (key: string, options?: { defaultValue?: string }) => string;

/**
 * One actionable line describing WHY a health check failed.
 *
 * The backend classifies every failure and reports the HTTP status, but the UI
 * used to render only the raw upstream message — so a wrong address (404) and a
 * dead key (401) produced the same red toast and the user could not tell which
 * half of their configuration to fix.
 *
 * The distinction that matters most: a 401/403 proves the address is correct and
 * the endpoint is enforcing auth. Saying so turns "it failed" into "your URL is
 * right, your key is not".
 */
const HEADLINE_KEYS: Record<ProviderHealthCheckErrorKind, { key: string; fallback: string }> = {
  unauthorized: {
    key: 'settings.health.unauthorized',
    fallback: '地址可达，但密钥被拒绝（401）',
  },
  forbidden: {
    key: 'settings.health.forbidden',
    fallback: '地址可达，但该密钥无权限（403）',
  },
  invalid_authorization_header: {
    key: 'settings.health.invalidAuthorizationHeader',
    fallback: '鉴权头格式不被该供应商接受',
  },
  not_found: {
    key: 'settings.health.notFound',
    fallback: '该地址不存在（404）——请检查 Base URL 与请求路径',
  },
  model_unavailable: {
    key: 'settings.health.modelUnavailable',
    fallback: '模型或推理接入点不可用——请核对精确 Model ID、开通状态与密钥权限',
  },
  non_api_response: {
    key: 'settings.health.nonApiResponse',
    fallback: '该地址返回的是网页而非 API——Base URL 很可能少了版本段',
  },
  insufficient_quota: {
    key: 'settings.health.insufficientQuota',
    fallback: '地址与密钥有效，但账户额度不足',
  },
  rate_limited: {
    key: 'settings.health.rateLimited',
    fallback: '地址与密钥有效，但触发了限流（429）',
  },
  timeout: { key: 'settings.health.timeout', fallback: '请求超时' },
  connection_error: {
    key: 'settings.health.connectionError',
    fallback: '无法建立连接——请检查网络、DNS 或代理',
  },
  aws_credentials: { key: 'settings.health.awsCredentials', fallback: 'AWS 凭据无效或缺失' },
  invalid_request: { key: 'settings.health.invalidRequest', fallback: '请求被供应商判定为非法' },
  api_error: { key: 'settings.health.apiError', fallback: '供应商返回了错误' },
  unknown: { key: 'settings.health.unknown', fallback: '失败原因未能归类' },
};

/** Does this failure prove the endpoint address itself is correct? */
export const endpointConfirmedByFailure = (
  kind: ProviderHealthCheckErrorKind | null | undefined
): boolean =>
  kind === 'unauthorized' ||
  kind === 'forbidden' ||
  kind === 'model_unavailable' ||
  kind === 'insufficient_quota' ||
  kind === 'rate_limited';

export const healthFailureHeadline = (
  t: Translate,
  result: Pick<ProviderHealthCheckResponse, 'error_kind' | 'http_status'>
): string => {
  const kind = result.error_kind;
  const entry = kind ? HEADLINE_KEYS[kind] : undefined;
  const headline = entry
    ? t(entry.key, { defaultValue: entry.fallback })
    : t('common.failed', { defaultValue: '失败' });
  // The status is only additive information; the kind already names the cause,
  // and for a document body the status is misleadingly 200.
  return result.http_status && !entry ? `${headline} (HTTP ${result.http_status})` : headline;
};
