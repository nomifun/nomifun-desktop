/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { mergeModelParams, splitModelParams } from './providerModelAdvanced';

describe('per-model advanced params split', () => {
  test('extracts quick fields and pretty-prints the rest', () => {
    const split = splitModelParams({ endpoint: '/custom/images', request_shape: 'multipart', size: '1024x1024' });
    expect(split.endpoint).toBe('/custom/images');
    expect(split.requestShape).toBe('multipart');
    expect(JSON.parse(split.restJson)).toEqual({ size: '1024x1024' });
  });

  test('degrades non-object params to an empty form', () => {
    expect(splitModelParams(null)).toEqual({ endpoint: '', requestShape: '', restJson: '' });
    expect(splitModelParams(undefined)).toEqual({ endpoint: '', requestShape: '', restJson: '' });
    expect(splitModelParams([1])).toEqual({ endpoint: '', requestShape: '', restJson: '' });
    expect(splitModelParams({ endpoint: 42 })).toEqual({ endpoint: '', requestShape: '', restJson: '' });
  });
});

describe('per-model advanced params merge', () => {
  test('quick fields win over same-named JSON keys', () => {
    const merged = mergeModelParams('{"endpoint": "/old", "size": "512x512"}', '/new', 'json');
    expect(merged).toEqual({
      ok: true,
      params: { size: '512x512', endpoint: '/new', request_shape: 'json' },
    });
  });

  test('empty quick fields remove their keys; empty editor yields bare quick params', () => {
    expect(mergeModelParams('{"endpoint": "/old", "request_shape": "json"}', '', '')).toEqual({
      ok: true,
      params: {},
    });
    expect(mergeModelParams('', '/v1/audio', '')).toEqual({
      ok: true,
      params: { endpoint: '/v1/audio' },
    });
    expect(mergeModelParams('', '', '')).toEqual({ ok: true, params: {} });
  });

  test('rejects invalid or non-object JSON', () => {
    expect(mergeModelParams('###', '', '')).toEqual({ ok: false, error: 'invalid_json' });
    expect(mergeModelParams('[1,2]', '', '')).toEqual({ ok: false, error: 'json_not_object' });
    expect(mergeModelParams('"str"', '', '')).toEqual({ ok: false, error: 'json_not_object' });
  });
});
