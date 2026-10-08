import { describe, expect, test } from 'bun:test';
import { healthFailureHeadline } from './healthFailureHeadline';

const fallback = (_key: string, options?: { defaultValue?: string }) => options?.defaultValue ?? _key;

describe('provider health failure headline', () => {
  test('503 is reported as temporary unavailability, not model or credential misconfiguration', () => {
    expect(healthFailureHeadline(fallback, { error_kind: 'api_error', http_status: 503 }))
      .toBe('供应商服务暂时不可用（503），请稍后重试');
  });

  test('other errors and specific reasons retain their existing classifications', () => {
    expect(healthFailureHeadline(fallback, { error_kind: 'api_error', http_status: 500 })).toBe('供应商返回了错误');
    expect(healthFailureHeadline(fallback, { error_kind: 'insufficient_quota', http_status: 503 })).toContain('账户额度不足');
    expect(healthFailureHeadline(fallback, { error_kind: 'rate_limited', http_status: 429 })).toContain('限流（429）');
    expect(healthFailureHeadline(fallback, { error_kind: 'forbidden', http_status: 403 })).toContain('无权限（403）');
    expect(healthFailureHeadline(fallback, { error_kind: undefined, http_status: 503 })).toBe('失败 (HTTP 503)');
  });
});
