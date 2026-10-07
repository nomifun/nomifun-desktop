import { expect, test } from 'bun:test';
import { localBrowserLink } from './localBrowserLink';

test('explicit loopback links retain their full navigation URL', () => {
  expect(localBrowserLink('http://localhost:5173/app?q=test#route')).toEqual({ url: 'http://localhost:5173/app?q=test#route' });
  expect(localBrowserLink('https://[::1]:8443/')).toEqual({ url: 'https://[::1]:8443/' });
  expect(localBrowserLink('http://127.0.0.2:3000/')).toEqual({ url: 'http://127.0.0.2:3000/' });
});

test('all-interface listening links map to loopback with source metadata', () => {
  expect(localBrowserLink('http://0.0.0.0:5173/path?a=1#x')).toEqual({ url: 'http://127.0.0.1:5173/path?a=1#x', mappedFrom: '0.0.0.0' });
  expect(localBrowserLink('http://[::]:3000/')).toEqual({ url: 'http://[::1]:3000/', mappedFrom: '[::]' });
});

test('no implicit local route for credentials, public hosts, relative or non-web URLs', () => {
  for (const href of ['http://localhost.evil.test/', 'http://localhost@evil.test/', 'http://evil.test@localhost/', 'http://192.168.1.1/', 'https://example.com/', '//localhost:3000/', 'localhost:3000', 'file:///tmp/test', 'javascript:alert(1)', 'http://localhost\\@evil.test', 'http://local\nhost/']) {
    expect(localBrowserLink(href)).toBeNull();
  }
});
