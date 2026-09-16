import '../../../../../../test/setup-dom.ts';
import React from 'react';
import { afterEach, expect, spyOn, test } from 'bun:test';
import { cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import type { IMessageToolCall } from '@/common/chat/chatLib';
import { parseConversationId } from '@/common/types/ids';
import * as platform from '@/renderer/utils/platform';
import browserWorkspace from '@/renderer/services/i18n/locales/en-US/browserWorkspace.json';
import zhBrowserWorkspace from '@/renderer/services/i18n/locales/zh-CN/browserWorkspace.json';
import LocalWebSearchResult from './LocalWebSearchResult';
import { localSearchResult } from './localSearchResultModel';
import ProcessTraceItem from './ProcessTraceItem';
import MessageToolCall from './MessageToolCall';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: {
  'en-US': { translation: { browserWorkspace } }, 'zh-CN': { translation: { browserWorkspace: zhBrowserWorkspace } },
}, interpolation: { escapeValue: false } });
const payload = {
  query: 'WebView2 documentation', provider: { kind: 'browser', id: 'nomi.local.browser', version: '1' },
  results: [{ citation_id: `nomi-local-search-${'a'.repeat(32)}`, rank: 1, title: 'WebView2 guide',
    url: 'https://example.com/docs?q=webview', snippet: 'A public source.' }],
};
const output = JSON.stringify(payload);
const input = JSON.stringify({ query: payload.query, limit: 3 });
const message: IMessageToolCall = {
  id: 'search-message', conversation_id: parseConversationId('0190f5fe-7c00-7a00-8000-000000000051'),
  type: 'tool_call', position: 'left', created_at: 1,
  content: { call_id: 'search-1', name: 'nomi_local_websearch', status: 'completed', args: { query: payload.query }, output },
};
function mount(element: React.ReactNode) {
  const screen = render(<I18nextProvider i18n={i18n}>{element}</I18nextProvider>);
  return { ...screen, page: within(screen.container) };
}
afterEach(async () => { cleanup(); await i18n.changeLanguage('en-US'); });

test('completed sources are readable and do not open anything until clicked', async () => {
  const open = spyOn(platform, 'openExternalUrl').mockResolvedValue();
  try {
    const { page } = mount(<LocalWebSearchResult input={input} output={output} state='completed' />);
    expect(page.getByText(payload.query)).toBeTruthy();
    expect(page.getByText('example.com')).toBeTruthy();
    expect(page.getByText('A public source.')).toBeTruthy();
    expect(page.getByRole('status').textContent).toBe('1 source');
    expect(open).not.toHaveBeenCalled();
    const link = page.getByRole('link', { name: 'WebView2 guide' });
    expect(link.getAttribute('href')).toBe(payload.results[0].url);
    expect(link.getAttribute('rel')).toBe('noopener noreferrer');
    fireEvent.click(link);
    await waitFor(() => expect(open).toHaveBeenCalledWith(payload.results[0].url));
    expect(open).toHaveBeenCalledTimes(1);
  } finally { open.mockRestore(); }
});

test('running and canceled calls never expose stale output as successful sources', () => {
  for (const state of ['running', 'canceled'] as const) {
    const { page, unmount } = mount(<LocalWebSearchResult input={input} output={output} state={state} />);
    expect(page.queryByRole('link')).toBeNull();
    expect(page.getByRole('status').textContent).toBe(browserWorkspace.search[state === 'running' ? 'searching' : 'canceled']);
    unmount();
  }
});

test('challenge and timeout are failures, not zero-result searches or raw diagnostics', () => {
  for (const [code, key] of [['NOMI_LOCAL_WEBSEARCH_CHALLENGE', 'challenge'], ['NOMI_LOCAL_WEBSEARCH_TIMEOUT', 'timeout'], ['NOMI_LOCAL_WEBSEARCH_BUSY', 'busy']] as const) {
    const { page, unmount } = mount(<LocalWebSearchResult input={input} output={JSON.stringify({ code, message: 'raw diagnostic sentinel' })} state='failed' />);
    expect(page.getByRole('status').textContent).toBe(browserWorkspace.search[key]);
    expect(page.queryByText(browserWorkspace.search.empty)).toBeNull();
    expect(page.queryByText('raw diagnostic sentinel')).toBeNull();
    expect(page.queryByRole('link')).toBeNull();
    unmount();
  }
});

test('malformed output is unavailable while a valid empty result is explicitly empty', () => {
  for (const [value, text] of [[output.slice(0, -2), browserWorkspace.search.invalid],
    [JSON.stringify({ ...payload, results: [] }), browserWorkspace.search.empty]]) {
    const { page, unmount } = mount(<LocalWebSearchResult output={value} state='completed' />);
    expect(page.getByRole('status').textContent).toBe(text);
    expect(page.queryByRole('link')).toBeNull();
    unmount();
  }
});

test('source text stays plain text and unsafe or credential-bearing URLs never become links', () => {
  for (const url of ['javascript:alert(1)', 'file:///C:/private', 'data:text/html,hello', 'https://user:pass@example.com', 'https://example.com/\npath', 'https:\\example.com']) {
    expect(localSearchResult(JSON.stringify({ ...payload, results: [{ ...payload.results[0], url }] }))).toBeNull();
  }
  const title = '<img src=x onerror=alert(1)>';
  const { page, container } = mount(<LocalWebSearchResult output={JSON.stringify({ ...payload, results: [{ ...payload.results[0], title }] })} state='completed' />);
  expect(page.getByRole('link').textContent).toBe(title);
  expect(container.querySelector('img')).toBeNull();
});

test('wrong provider, duplicate citations and oversized results fail closed', () => {
  expect(localSearchResult(JSON.stringify({ ...payload, provider: { ...payload.provider, id: 'other' } }))).toBeNull();
  expect(localSearchResult(JSON.stringify({ ...payload, results: [payload.results[0], payload.results[0]] }))).toBeNull();
  expect(localSearchResult(JSON.stringify({ ...payload, results: Array(11).fill(payload.results[0]) }))).toBeNull();
  expect(localSearchResult(' '.repeat(65537))).toBeNull();
});

test('an external-open failure remains in the tool message and the link can be retried', async () => {
  const open = spyOn(platform, 'openExternalUrl').mockRejectedValueOnce(new Error('fixture')).mockResolvedValue();
  try {
    const { page } = mount(<LocalWebSearchResult output={output} state='completed' />);
    fireEvent.click(page.getByRole('link'));
    expect((await page.findByRole('alert')).textContent).toBe(browserWorkspace.search.openFailed);
    fireEvent.click(page.getByRole('link'));
    await waitFor(() => expect(open).toHaveBeenCalledTimes(2));
    expect(page.queryByRole('alert')).toBeNull();
  } finally { open.mockRestore(); }
});

test('both normal conversation trace layouts and direct tool messages render the source list', () => {
  for (const node of [<ProcessTraceItem item={message} variant='list' />, <ProcessTraceItem item={message} variant='receipt' />,
    <MessageToolCall message={message} />]) {
    const { page, unmount } = mount(node);
    expect(page.getByRole('link', { name: 'WebView2 guide' })).toBeTruthy();
    unmount();
  }
});

test('source presentation is localized in Chinese', async () => {
  await i18n.changeLanguage('zh-CN');
  const { page } = mount(<LocalWebSearchResult output={output} state='completed' />);
  expect(page.getByRole('region', { name: 'Nomi 本地网页搜索' })).toBeTruthy();
  expect(page.getByRole('status').textContent).toBe('1 个来源');
});
