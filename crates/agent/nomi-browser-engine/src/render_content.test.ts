import { expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { createContext, runInContext } from 'node:vm';

const script = readFileSync(new URL('./render_content.js', import.meta.url), 'utf8');
function fixture(html: string, readyState = 'complete') {
  let now = 0;
  let mutated = () => {};
  const document = { readyState, documentElement: { outerHTML: html } };
  const context = createContext({ document, location: { href: 'https://example.test/rendered' },
    performance: { now: () => now }, TextEncoder, TextDecoder,
    MutationObserver: class { constructor(callback: () => void) { mutated = callback; } observe() {} },
  });
  return { read: () => runInContext(script, context), time: (value: number) => { now = value; },
    change: (html: string) => { document.documentElement.outerHTML = html; mutated(); } };
}
test('render capture waits for the document and quiet DOM before returning actual HTML', () => {
  expect(fixture('<p>loading</p>', 'loading').read().state).toBe('waiting');
  const page = fixture('<p>initial</p>');
  expect(page.read().state).toBe('waiting');
  page.time(500); page.change('<p>动态内容</p>');
  expect(page.read().state).toBe('waiting');
  page.time(800);
  expect(page.read()).toEqual({ state: 'ready', final_url: 'https://example.test/rendered', html: '<p>动态内容</p>', html_truncated: false });
});
test('render capture truncates UTF-8 without inventing a replacement character', () => {
  const page = fixture('界'.repeat(100000));
  page.read(); page.time(1000);
  const result = page.read();
  expect(result.html_truncated).toBe(true);
  expect(new TextEncoder().encode(result.html).length).toBeLessThanOrEqual(256 * 1024);
  expect(result.html).not.toContain('�');
  expect(result.html).toBe('界'.repeat(Math.floor(256 * 1024 / 3)));
});
test('continuously changing pages still produce a bounded point-in-time snapshot', () => {
  const page = fixture('<p>0</p>'); page.read();
  page.time(1900); page.change('<p>1</p>');
  expect(page.read().state).toBe('waiting');
  page.time(2100); page.change('<p>2</p>');
  expect(page.read().html).toBe('<p>2</p>');
});
