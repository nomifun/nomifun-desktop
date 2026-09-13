import { describe, expect, test } from 'bun:test';
import { BackendHttpError } from '@/common/adapter/httpBridge';
import { productAgentErrorReason, productSelectionValue } from './ProductAgentBindingSelect';

describe('product Agent selection', () => {
  test('keeps template intent separate from personal Agent identity', () => {
    expect(productSelectionValue({ kind: 'template', template_key: 'companion.default' })).toBe('template:companion.default');
    expect(productSelectionValue({ kind: 'preset', preset_id: 'custom' })).toBe('preset:custom');
  });
  test('maps server errors to safe localized categories without exposing the envelope', () => {
    const error = (code: string, status: number) => new BackendHttpError({
      method: 'PUT', path: '/api/product-agent-bindings/companion/test', status,
      body: { success: false, code, error: 'provider/private-model internal failure' },
    });
    expect(productAgentErrorReason(error('MODEL_ROUTE_FEATURES_MISSING', 422))).toBe('modelChanged');
    expect(productAgentErrorReason(error('REMOTE_SESSION_BUSY', 409))).toBe('busy');
    expect(productAgentErrorReason(error('INTERNAL_ERROR', 500))).toBe('saveFailed');
    expect(productAgentErrorReason(new Error('fetch failed'))).toBe('saveFailed');
  });
});
