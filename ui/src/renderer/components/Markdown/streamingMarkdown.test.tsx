import '../../../../test/setup-dom.ts';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { Message } from '@arco-design/web-react';
import { afterEach, expect, spyOn, test } from 'bun:test';
import { MemoryRouter } from 'react-router-dom';
import { createInstance } from 'i18next';
import { initReactI18next } from 'react-i18next';
import { BrowserLinkContext } from '@/renderer/pages/conversation/Browser/BrowserLinkContext';
import type { LocalBrowserLink } from '@/renderer/pages/conversation/Browser/localBrowserLink';
import * as platform from '@/renderer/utils/platform';
import * as clipboard from '@/renderer/utils/ui/clipboard';
import MarkdownView from './index';
import { createStreamingMarkdown } from './streamingMarkdown';

await createInstance().use(initReactI18next).init({ lng: 'en-US', resources: {} });
afterEach(cleanup);

const article = (container: HTMLElement) => container.querySelector('.markdown-shadow')!.shadowRoot!.querySelector('.markdown-article')!;

test('native Markdown escaped brackets stay literal in static and streamed rendering', () => {
  const source = '\\[内容\\] and ' + String.raw`C:\\Users\\test\[draft\]\\file\(1\).md`;
  const expected = '[内容] and ' + String.raw`C:\Users\test[draft]\file(1).md`;
  const view = (revealOffset?: number) => <MemoryRouter><MarkdownView normalizeLatexDelimiters={false} revealOffset={revealOffset}>{source}</MarkdownView></MemoryRouter>;
  const page = render(view());
  expect(article(page.container).querySelector('p')?.textContent).toBe(expected);
  expect(article(page.container).querySelector('.katex')).toBeNull();
  page.rerender(view(source.length));
  expect(article(page.container).querySelector('p')?.textContent).toBe(expected);
  expect(article(page.container).querySelector('.katex')).toBeNull();
});

test('reveal frames reuse one full-source parse and keep completed DOM blocks mounted', () => {
  let parses = 0;
  const countParse = () => () => { parses += 1; };
  const source = 'First paragraph.\n\nSecond paragraph with **strong text**.';
  const markdown = createStreamingMarkdown(source, source, { rehypePlugins: [countParse] });
  const page = render(<>{markdown.render(3)}</>);
  expect(page.container.textContent).toBe('Fir\n');
  const first = page.container.querySelector('p');
  page.rerender(<>{markdown.render(25)}</>);
  expect(page.container.querySelector('p')).toBe(first);
  expect(page.container.textContent).toContain('Second ');
  expect(page.container.textContent).not.toContain('strong text');
  page.rerender(<>{markdown.render(source.length)}</>);
  expect(page.container.querySelector('p')).toBe(first);
  expect(page.container.querySelector('strong')?.textContent).toBe('strong text');
  expect(parses).toBe(1);
});

test('streamed prose retains list, table, fence, reference and math semantics at settlement', () => {
  const source = '[Reference][docs]\n\n- First item\n- Second item\n\n| A | B |\n| - | - |\n| Cell one | Cell two |\n\n```ts\nconst result = 1;\n```\n\nMath \\(x+1\\).\n\n[docs]: https://example.com/docs';
  const view = (offset?: number) => <MemoryRouter><MarkdownView revealOffset={offset}>{source}</MarkdownView></MemoryRouter>;
  const page = render(view(source.indexOf('Second item') + 3));
  const host = page.container.querySelector('.markdown-shadow');
  expect(article(page.container).querySelectorAll('li')).toHaveLength(2);
  expect(article(page.container).textContent).toContain('Sec');
  expect(article(page.container).textContent).not.toContain('Cell one');
  expect(article(page.container).querySelector('pre, code, .katex')).toBeNull();
  page.rerender(view(source.indexOf('Cell two') + 4));
  expect(article(page.container).querySelector('table')).not.toBeNull();
  expect(article(page.container).textContent).not.toContain('const result');
  page.rerender(view(source.length));
  expect(page.container.querySelector('.markdown-shadow')).toBe(host);
  expect(article(page.container).querySelector('a')?.getAttribute('href')).toBe('https://example.com/docs');
  expect(article(page.container).querySelector('.katex')).not.toBeNull();
  const streamed = article(page.container).innerHTML;
  page.rerender(view());
  expect(article(page.container).innerHTML).toBe(streamed);
});

test('incomplete links reveal inert labels and completed syntax keeps routing and image delivery policy', () => {
  const source = 'Start [Preview](http://localhost:5173/result) ![private](https://example.com/image.png) End';
  const opened: LocalBrowserLink[] = [];
  const view = (offset: number) => <MemoryRouter><BrowserLinkContext.Provider value={link => opened.push(link)}>
    <MarkdownView revealOffset={offset} allowUnverifiedImages={false}>{source}</MarkdownView>
  </BrowserLinkContext.Provider></MemoryRouter>;
  const page = render(view(source.indexOf('Preview') + 3));
  expect(article(page.container).querySelector('a, img')).toBeNull();
  expect(article(page.container).textContent).toBe('Start Pre');
  expect(article(page.container).querySelector('[href], [data-markdown-href]')).toBeNull();
  fireEvent.click(article(page.container).querySelector('span')!);
  expect(opened).toEqual([]);
  expect(article(page.container).textContent).not.toContain('http');
  page.rerender(view(source.indexOf(' ![private]')));
  const link = article(page.container).querySelector('a')!;
  fireEvent.click(link);
  expect(opened).toEqual([{ url: 'http://localhost:5173/result' }]);
  expect(article(page.container).textContent).not.toContain('image.png');
  page.rerender(view(source.length));
  expect(article(page.container).querySelector('img')).toBeNull();
  expect(article(page.container).textContent).toContain('Unverified image reference: private');
});

test('a long autolink reveals its visible URL progressively before it becomes clickable', () => {
  const source = 'See https://example.com/a/very/long/path now.';
  const view = (offset: number) => <MemoryRouter><MarkdownView revealOffset={offset}>{source}</MarkdownView></MemoryRouter>;
  const first = source.indexOf('/a/');
  const page = render(view(first));
  expect(article(page.container).textContent).toBe(source.slice(0, first));
  expect(article(page.container).querySelector('a, [href], [data-markdown-href]')).toBeNull();
  page.rerender(view(first + 8));
  expect(article(page.container).textContent).toBe(source.slice(0, first + 8));
  expect(article(page.container).querySelector('a')).toBeNull();
  page.rerender(view(source.indexOf(' now.')));
  expect(article(page.container).querySelector('a')?.getAttribute('href')).toBe('https://example.com/a/very/long/path');
  expect(article(page.container).textContent).not.toContain('now');
});

test('an EOF-terminated code fence stays mounted on received appends and copies the complete received code', async () => {
  const first = 'Intro.\n\n```ts\nconst first = 1;';
  const next = first + '\nconst second = 2;';
  const copy = spyOn(clipboard, 'copyText').mockResolvedValue(undefined);
  const toast = spyOn(Message, 'success').mockReturnValue(() => {});
  const view = (source: string, offset: number) => <MemoryRouter><MarkdownView revealOffset={offset}>{source}</MarkdownView></MemoryRouter>;
  try {
    const page = render(view(first, first.indexOf('```') - 1));
    expect(article(page.container).querySelector('pre')).toBeNull();
    page.rerender(view(first, first.indexOf('```')));
    const pre = article(page.container).querySelector('pre');
    expect(pre).not.toBeNull();
    expect(pre?.textContent).toContain('const first = 1;');
    page.rerender(view(first, first.length));
    page.rerender(view(next, first.length));
    expect(article(page.container).querySelector('pre')).toBe(pre);
    expect(pre?.textContent).toContain('const second = 2;');
    await act(async () => fireEvent.click(pre!.querySelector('[title="common.copy"]')!.firstElementChild!));
    expect(copy).toHaveBeenCalledWith('const first = 1;\nconst second = 2;');
    page.rerender(view(next + '\n```\n\nFollowing prose.', first.length));
    expect(article(page.container).querySelector('pre')).toBe(pre);
    expect(article(page.container).textContent).not.toContain('Following prose');
  } finally { copy.mockRestore(); toast.mockRestore(); }
});

test('EOF-terminated block math remains mounted on appends while later prose stays paced', () => {
  const first = '$$\nx';
  const next = first + '+1';
  const view = (source: string, offset: number) => <MemoryRouter><MarkdownView revealOffset={offset}>{source}</MarkdownView></MemoryRouter>;
  const page = render(view(first, first.length));
  const math = article(page.container).querySelector('.katex-display');
  expect(math).not.toBeNull();
  page.rerender(view(next, first.length));
  expect(article(page.container).querySelector('.katex-display')).toBe(math);
  expect(math?.querySelector('annotation')?.textContent).toBe('x+1');
  page.rerender(view(next + '\n$$\n\nFollowing prose.', first.length));
  expect(article(page.container).querySelector('.katex-display')).toBe(math);
  expect(article(page.container).textContent).not.toContain('Following prose');
});

test('reveal uses original UTF-16 positions through escapes, entities, emoji and LaTeX conversion', () => {
  const source = 'A &amp; B \\* C 😀 Math \\(x+1\\) after.';
  const view = (offset: number) => <MemoryRouter><MarkdownView revealOffset={offset}>{source}</MarkdownView></MemoryRouter>;
  const page = render(view(source.indexOf('&amp;') + 3));
  expect(article(page.container).textContent).toBe('A ');
  page.rerender(view(source.indexOf(' B')));
  expect(article(page.container).textContent).toBe('A &');
  page.rerender(view(source.indexOf('😀') + 1));
  expect(article(page.container).textContent).toBe('A & B * C ');
  page.rerender(view(source.indexOf('x+1') + 1));
  expect(article(page.container).querySelector('.katex')).toBeNull();
  page.rerender(view(source.indexOf(' after.')));
  expect(article(page.container).querySelector('.katex')).not.toBeNull();
  expect(article(page.container).textContent).not.toContain('after');
});

test('stream reveal retains external URL handling and dangerous URL filtering', async () => {
  const external = spyOn(platform, 'openExternalUrl').mockResolvedValue();
  const source = '[Docs](https://example.com/docs) [Unsafe](javascript:alert)';
  try {
    const page = render(<MemoryRouter><MarkdownView revealOffset={source.length}>{source}</MarkdownView></MemoryRouter>);
    const links = article(page.container).querySelectorAll('a');
    fireEvent.click(links[0]);
    expect(external).toHaveBeenCalledWith('https://example.com/docs');
    expect(links[1].getAttribute('href')).toBe('');
  } finally { external.mockRestore(); }
});
