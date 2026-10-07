import '../../../../test/setup-dom.ts';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, expect, spyOn, test } from 'bun:test';
import { MemoryRouter } from 'react-router-dom';
import { BrowserLinkContext } from '@/renderer/pages/conversation/Browser/BrowserLinkContext';
import type { LocalBrowserLink } from '@/renderer/pages/conversation/Browser/localBrowserLink';
import * as platform from '@/renderer/utils/platform';
import MarkdownView from './index';
import { createInstance } from 'i18next';
import { initReactI18next } from 'react-i18next';

await createInstance().use(initReactI18next).init({ lng: 'en-US', resources: {} });

async function clickMarkdownLink(container: HTMLElement, name: string) {
  // Production Markdown is rendered through a React portal into Shadow DOM.
  // Query the real roots; don't replace ShadowView with a light-DOM mock.
  const link = await waitFor(() => {
    const links = Array.from(container.querySelectorAll('.markdown-shadow')).flatMap(host => Array.from(host.shadowRoot?.querySelectorAll('a') ?? []));
    const result = links.find(element => element.textContent === name);
    expect(result).toBeDefined();
    return result!;
  });
  fireEvent.click(link);
}

afterEach(cleanup);

test('a real Markdown click opens only its own conversation browser context', async () => {
  const first: LocalBrowserLink[] = [], second: LocalBrowserLink[] = [];
  const screen = render(<MemoryRouter>
    <BrowserLinkContext.Provider value={link => first.push(link)}>
      <MarkdownView>{'[First preview](http://localhost:5173/first?q=1#route)'}</MarkdownView>
    </BrowserLinkContext.Provider>
    <BrowserLinkContext.Provider value={link => second.push(link)}>
      <MarkdownView>{'[Second preview](http://0.0.0.0:3000/second)'}</MarkdownView>
    </BrowserLinkContext.Provider>
  </MemoryRouter>);
  await clickMarkdownLink(screen.container, 'First preview');
  expect(first).toEqual([{ url: 'http://localhost:5173/first?q=1#route' }]);
  expect(second).toEqual([]);
  await clickMarkdownLink(screen.container, 'Second preview');
  expect(first).toHaveLength(1);
  expect(second).toEqual([{ url: 'http://127.0.0.1:3000/second', mappedFrom: '0.0.0.0' }]);
});

test('WebUI or a surface without a browser owner keeps external navigation', async () => {
  const external = spyOn(platform, 'openExternalUrl').mockResolvedValue();
  try {
    const screen = render(<MemoryRouter><MarkdownView>{'[Preview](http://localhost:5173/)'}</MarkdownView></MemoryRouter>);
    await clickMarkdownLink(screen.container, 'Preview');
    await waitFor(() => expect(external).toHaveBeenCalledWith('http://localhost:5173/'));
  } finally { external.mockRestore(); }
});

test('a public link is not diverted into the local preview route', async () => {
  const local: LocalBrowserLink[] = [];
  const external = spyOn(platform, 'openExternalUrl').mockResolvedValue();
  try {
    const screen = render(<MemoryRouter><BrowserLinkContext.Provider value={link => local.push(link)}>
      <MarkdownView>{'[Documentation](https://example.com/docs)'}</MarkdownView>
    </BrowserLinkContext.Provider></MemoryRouter>);
    await clickMarkdownLink(screen.container, 'Documentation');
    expect(local).toEqual([]);
    await waitFor(() => expect(external).toHaveBeenCalledWith('https://example.com/docs'));
  } finally { external.mockRestore(); }
});
