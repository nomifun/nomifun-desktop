import { redactSensitiveText } from '../adapter/httpBridge';

export const MODEL_FAILURE_REASONS = [
  'auth_failed', 'invalid_key', 'expired_key', 'insufficient_quota', 'insufficient_balance',
  'subscription_expired', 'model_not_in_plan', 'permission_denied', 'model_permission_denied',
  'endpoint_missing', 'model_not_found', 'non_api_response', 'auth_scheme_mismatch',
  'billing_required', 'rate_limited', 'dns_failure', 'connection_failed', 'tls_failure',
  'proxy_failure', 'request_timeout', 'upstream_server_error', 'provider_unavailable',
  'network_failure', 'stream_interrupted', 'invalid_response', 'invalid_request',
  'unsupported_feature', 'content_policy', 'prompt_too_long', 'provider_overloaded',
  'configuration_error', 'invalid_endpoint', 'credentials_missing', 'credential_target_mismatch',
  'spend_limit_reached',
] as const;
export type ModelFailureReason = (typeof MODEL_FAILURE_REASONS)[number];

/** The backend's captured request evidence, never reconstructed from transcript text. */
export interface ModelFailureDiagnostic {
  reason: ModelFailureReason;
  httpStatus?: number;
  providerId?: string;
  modelName?: string;
  providerCode?: string;
  providerType?: string;
  providerParam?: string;
  endpoint?: string;
  requestId?: string;
  retryAfterMs?: number;
  protocol?: string;
  authScheme?: string;
  contentType?: string;
  transportDetail?: string;
}

const credentialLike = (value: string) => /^(?:sk[-_]|AIza|AKIA|ASIA)/.test(value);

const machineValue = (value: unknown): string | undefined => {
  if (typeof value !== 'string' || value.length > 200 || credentialLike(value)) return undefined;
  const safe = redactSensitiveText(value);
  return /^[a-z0-9_./:+#-]+$/i.test(safe) ? safe : undefined;
};

export function diagnosticEndpoint(value: unknown): string | undefined {
  if (typeof value !== 'string' || value.length > 4096) return undefined;
  try {
    const url = new URL(value);
    if (!['http:', 'https:', 'ws:', 'wss:'].includes(url.protocol)) return undefined;
    const path = url.pathname.split('/').map(part => credentialLike(part) ? '[REDACTED]' : part).join('/');
    return `${url.origin}${path}`;
  } catch { return undefined; }
}

const safeTransportDetail = (value: unknown): string | undefined => {
  if (typeof value !== 'string' || !value.trim()) return undefined;
  return redactSensitiveText(value.slice(0, 2000))
    .replace(/((?:proxy-authorization|authorization|x-api-key|api[_ -]?key)\s*:\s*)[^\r\n]+/gi, '$1[REDACTED]')
    .replace(/(?:wss?|https?):\/\/[^\s"'<>]+/gi, url => diagnosticEndpoint(url) ?? '[REDACTED_URL]');
};

export function normalizeModelFailureDiagnostic(value: unknown): ModelFailureDiagnostic | undefined {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return undefined;
  const data = value as Record<string, unknown>;
  if (!MODEL_FAILURE_REASONS.includes(data.reason as ModelFailureReason)) return undefined;
  const result: ModelFailureDiagnostic = { reason: data.reason as ModelFailureReason };
  if (Number.isInteger(data.httpStatus) && Number(data.httpStatus) >= 100 && Number(data.httpStatus) <= 599) {
    result.httpStatus = data.httpStatus as number;
  }
  if (Number.isSafeInteger(data.retryAfterMs) && Number(data.retryAfterMs) >= 0) result.retryAfterMs = data.retryAfterMs as number;
  for (const key of ['providerId', 'providerCode', 'providerType', 'requestId', 'protocol', 'authScheme'] as const) {
    const safe = machineValue(data[key]);
    if (safe) result[key] = safe;
  }
  if (typeof data.providerParam === 'string' && data.providerParam.length <= 256
    && /^[a-z0-9_.$/~:[\]-]+$/i.test(data.providerParam) && !credentialLike(data.providerParam)) result.providerParam = data.providerParam;
  const endpoint = diagnosticEndpoint(data.endpoint);
  if (endpoint) result.endpoint = endpoint;
  if (typeof data.modelName === 'string' && data.modelName.length <= 512 && data.modelName.trim()
    && !/[\x00-\x1f\x7f]/.test(data.modelName) && !credentialLike(data.modelName)) result.modelName = redactSensitiveText(data.modelName);
  if (typeof data.contentType === 'string' && data.contentType.length <= 128
    && !/[\x00-\x1f\x7f]/.test(data.contentType)) result.contentType = redactSensitiveText(data.contentType);
  const detail = safeTransportDetail(data.transportDetail);
  if (detail) result.transportDetail = detail;
  return result;
}
