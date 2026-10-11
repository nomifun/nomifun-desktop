import { expect, test } from 'bun:test';
import { buildMobilePairingUrl } from './connections';

test('Mobile pairing wraps the dedicated credential exchange instead of browser login', () => {
  const outer = new URL(buildMobilePairingUrl('https://work.example.net', 'token+/=&'));
  expect(outer.protocol).toBe('nomifun:');
  expect(outer.hostname).toBe('pair');
  expect(outer.searchParams.get('v')).toBe('1');
  const inner = new URL(outer.searchParams.get('url')!);
  expect(inner.origin).toBe('https://work.example.net');
  expect(inner.pathname).toBe('/device-pair');
  expect(inner.searchParams.get('token')).toBe('token+/=&');
});
