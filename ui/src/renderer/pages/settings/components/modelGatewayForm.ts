/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/** A gateway API root cannot carry credentials, query parameters or fragments. */
export function gatewayAddress(value: string): { root: string; domain: string; insecure: boolean } | undefined {
  try {
    const url = new URL(value.trim());
    if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password || url.search || url.hash) return;
    const path = url.pathname.replace(/\/+$/, '');
    if (path !== '' && path !== '/v1') return;
    url.pathname = '/';
    const local = ['localhost', '127.0.0.1', '[::1]'].includes(url.hostname.toLowerCase());
    return { root: url.toString().replace(/\/$/, ''), domain: url.host, insecure: url.protocol === 'http:' && !local };
  } catch { return; }
}

/** External account links require an absolute HTTPS URL without embedded credentials. */
export function gatewayExternalUrl(value: unknown): string | undefined {
  if (typeof value !== 'string' || !/^https:\/\//i.test(value)) return;
  try {
    const url = new URL(value);
    if (url.protocol !== 'https:' || url.username || url.password || url.hash) return;
    for (const key of url.searchParams.keys()) {
      if (/(^|[_-])(key|token|authorization|secret|password|credential)($|[_-])/i.test(key)) return;
    }
    return url.toString();
  } catch { return; }
}

/** Financial DTOs use decimal strings. Only exact safe numbers are accepted by local fixtures. */
function exactGatewayInteger(value: string | number): bigint {
  if (typeof value === 'number' && !Number.isSafeInteger(value)) throw new RangeError('Gateway amount must be an exact integer');
  if (typeof value === 'string' && !/^-?(0|[1-9][0-9]*)$/.test(value)) throw new RangeError('Gateway amount must be a decimal integer');
  const integer = BigInt(value);
  if (integer < -(2n ** 63n) || integer > 2n ** 63n - 1n) throw new RangeError('Gateway amount is outside int64');
  return integer;
}

/** Display minor currency units with exact integer arithmetic, including values beyond 2^53. */
export function gatewayMoney(amount: string | number, currency: string, locale: string): string {
  const integer = exactGatewayInteger(amount);
  const format = new Intl.NumberFormat(locale, { style: 'currency', currency });
  const digits = format.resolvedOptions().maximumFractionDigits ?? 2;
  const divisor = 10n ** BigInt(digits);
  const magnitude = integer < 0n ? -integer : integer;
  const major = magnitude / divisor;
  const minor = magnitude % divisor;
  // BigInt has no negative zero. Use a negative template and replace its integer
  // part when a negative amount is smaller than one major currency unit.
  const negativeZero = integer < 0n && major === 0n;
  const parts = format.formatToParts(integer < 0n ? -(major || 1n) : major);
  const digitFormat = new Intl.NumberFormat(locale, { useGrouping: false, maximumFractionDigits: 0 });
  const fractionFormat = new Intl.NumberFormat(locale, { useGrouping: false, minimumIntegerDigits: Math.max(1, digits), maximumFractionDigits: 0 });
  return parts.map((part) => {
    if (part.type === 'integer' && negativeZero) return digitFormat.format(0n);
    if (part.type === 'fraction') return fractionFormat.format(minor);
    return part.value;
  }).join('');
}

export function gatewayInteger(value: string | number | null | undefined, locale: string, unknown: string): string {
  return value == null ? unknown : exactGatewayInteger(value).toLocaleString(locale);
}

export function gatewayDate(value: string | null | undefined, locale: string, unknown: string): string {
  if (!value) return unknown;
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? unknown : date.toLocaleString(locale);
}
