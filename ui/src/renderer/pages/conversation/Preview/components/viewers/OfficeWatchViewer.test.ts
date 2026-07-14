import { describe, expect, it } from 'vitest';

import { resolveOfficeWatchUrl } from './OfficeWatchViewer';

const CAPABILITY = '0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef';
const OTHER_CAPABILITY = 'abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789';

describe('resolveOfficeWatchUrl', () => {
  for (const [docType, prefix] of [
    ['word', 'office-watch-proxy'],
    ['excel', 'office-watch-proxy'],
    ['ppt', 'ppt-proxy'],
  ] as const) {
    it(`keeps the ${docType} capability path on every supported shell`, () => {
      const path = `/api/${prefix}/${CAPABILITY}/assets/index.js`;

      expect(resolveOfficeWatchUrl(path, CAPABILITY, docType, 'http://127.0.0.1:17420/')).toBe(
        `http://127.0.0.1:17420${path}`
      );
    });
  }

  it('preserves the trailing slash required for relative iframe assets', () => {
    const path = `/api/office-watch-proxy/${CAPABILITY}/`;

    expect(resolveOfficeWatchUrl(path, CAPABILITY, 'word', '')).toBe(path);
  });

  for (const [url, capability, docType] of [
    ['/api/office-watch-proxy/43210/', CAPABILITY, 'word'],
    [`/api/ppt-proxy/${CAPABILITY}/`, CAPABILITY, 'word'],
    [`/api/office-watch-proxy/${CAPABILITY}/`, OTHER_CAPABILITY, 'excel'],
    [`http://127.0.0.1:43210/`, CAPABILITY, 'word'],
    [`/api/office-watch-proxy/${CAPABILITY.toUpperCase()}/`, CAPABILITY.toUpperCase(), 'word'],
    [`/api/office-watch-proxy/${CAPABILITY}`, CAPABILITY, 'word'],
    [`/api/office-watch-proxy/${CAPABILITY}/%2e%2e/other`, CAPABILITY, 'word'],
  ] as const) {
    it(`fails closed for an untrusted preview URL: ${url}`, () => {
      expect(() => resolveOfficeWatchUrl(url, capability, docType, 'http://localhost:17420')).toThrow(
        'Invalid Office preview capability URL'
      );
    });
  }
});
