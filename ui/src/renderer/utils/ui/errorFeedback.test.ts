/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { BackendHttpError } from '@/common/adapter/httpBridge';
import { getErrorTechnicalDetails } from './errorFeedback';

describe('getErrorTechnicalDetails', () => {
  test('preserves error names and messages while redacting credentials', () => {
    const failure = new Error('request /generate?token=query-secret failed: Bearer bearer-secret');
    failure.name = 'GenerationError';

    expect(getErrorTechnicalDetails(failure)).toBe(
      'GenerationError: request /generate?token=[REDACTED] failed: Bearer [REDACTED]'
    );
    expect(getErrorTechnicalDetails('request /generate?api_key=key-secret failed')).toBe(
      'request /generate?api_key=[REDACTED] failed'
    );
  });

  test('preserves redacted bridge diagnostics including cross-module bridge errors', () => {
    const failure = new BackendHttpError({
      method: 'POST',
      path: '/api/templates/run?token=query-secret',
      status: 500,
      body: { code: 'INTERNAL_ERROR', error: 'Bearer bearer-secret', details: { api_key: 'key-secret' } },
    });
    const details = getErrorTechnicalDetails(failure);

    expect(details).toContain('BackendHttpError: Backend POST /api/templates/run');
    expect(details).toContain('[REDACTED]');
    expect(details).not.toContain('query-secret');
    expect(details).not.toContain('bearer-secret');
    expect(details).not.toContain('key-secret');
    expect(getErrorTechnicalDetails({
      name: 'BackendHttpError', status: 500, code: 'INTERNAL_ERROR', message: 'Bearer cross-module-secret',
    })).toBe('BackendHttpError: Bearer [REDACTED]');
  });

  test('never serializes arbitrary thrown objects', () => {
    const unknown = {
      password: 'private-secret',
      name: 'Error',
      message: 'unknown object message',
      toJSON() { throw new Error('must not serialize'); },
      toString() { throw new Error('must not stringify'); },
    };

    for (const value of [unknown, ['private-secret'], null, undefined, 42, true]) {
      expect(getErrorTechnicalDetails(value)).toBe('Unknown error');
    }
  });
});
