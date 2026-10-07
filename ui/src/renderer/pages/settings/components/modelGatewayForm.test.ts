/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */
import { describe, expect, test } from 'bun:test';
import { gatewayAddress, gatewayDate, gatewayExternalUrl, gatewayInteger, gatewayMoney } from './modelGatewayForm';

describe('gateway address and external links', () => {
  test('normalizes gateway roots and flags only remote HTTP', () => {
    expect(gatewayAddress('https://gateway.example/v1/')).toEqual({ root: 'https://gateway.example', domain: 'gateway.example', insecure: false });
    for (const host of ['localhost', '127.0.0.1', '[::1]']) expect(gatewayAddress(`http://${host}:9090`)?.insecure).toBe(false);
    expect(gatewayAddress('http://gateway.example')?.insecure).toBe(true);
  });
  test('rejects URL credentials instead of forwarding them', () => {
    for (const url of ['https://user:key@gateway.example', 'https://gateway.example?key=test', 'https://gateway.example#key', '/relative', 'https://gateway.example/proxy/v1', 'file:///tmp/gateway']) expect(gatewayAddress(url)).toBeUndefined();
  });
  test('purchase and console links are absolute HTTPS without credential transport', () => {
    expect(gatewayExternalUrl('https://operator.example/buy?plan=pro')).toBe('https://operator.example/buy?plan=pro');
    for (const url of ['http://operator.example/buy', '/buy', '//operator.example/buy', 'javascript:alert(1)', 'https://user:pass@operator.example', 'https://operator.example?api_key=secret', 'https://operator.example?token=secret', 'https://operator.example?client_secret=secret', 'https://operator.example#secret']) expect(gatewayExternalUrl(url)).toBeUndefined();
  });
  test('uses currency fraction metadata for minor units and never invents date bounds', () => {
    expect(gatewayMoney(12345, 'USD', 'en-US')).toBe('$123.45');
    expect(gatewayMoney(12345, 'JPY', 'en-US')).toBe('¥12,345');
    expect(gatewayDate(null, 'en-US', 'Not disclosed')).toBe('Not disclosed');
    expect(gatewayDate('invalid', 'en-US', 'Not disclosed')).toBe('Not disclosed');
  });
  test('formats int64 money exactly beyond the JavaScript safe integer boundary', () => {
    expect(gatewayMoney('9007199254740993', 'USD', 'en-US')).toBe('$90,071,992,547,409.93');
    expect(gatewayMoney('9223372036854775807', 'USD', 'en-US')).toBe('$92,233,720,368,547,758.07');
    expect(gatewayMoney('-9223372036854775808', 'USD', 'en-US')).toBe('-$92,233,720,368,547,758.08');
    expect(gatewayMoney('9223372036854775807', 'JPY', 'en-US')).toBe('¥9,223,372,036,854,775,807');
    expect(gatewayMoney('-1', 'USD', 'en-US')).toBe('-$0.01');
    expect(gatewayMoney('-1', 'CNY', 'zh-CN')).toBe('-¥0.01');
    expect(gatewayMoney('1001', 'BHD', 'en-US')).toBe('BHD 1.001');
  });
  test('formats quotas exactly and preserves undisclosed nullable limits', () => {
    expect(gatewayInteger('9007199254740993', 'en-US', 'Not disclosed')).toBe('9,007,199,254,740,993');
    expect(gatewayInteger('9223372036854775807', 'en-US', 'Not disclosed')).toBe('9,223,372,036,854,775,807');
    expect(gatewayInteger('0', 'en-US', 'Not disclosed')).toBe('0');
    expect(gatewayInteger(null, 'en-US', 'Not disclosed')).toBe('Not disclosed');
  });
  test('rejects unsafe or fractional numbers and malformed/out-of-range decimal amounts', () => {
    for (const amount of [9007199254740993, 1.5, Number.NaN, Number.POSITIVE_INFINITY, '1.5', '1e3', '9223372036854775808', '-9223372036854775809']) {
      expect(() => gatewayMoney(amount, 'USD', 'en-US')).toThrow(RangeError);
      expect(() => gatewayInteger(amount, 'en-US', 'Not disclosed')).toThrow(RangeError);
    }
  });
});
