import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter, useLocation } from 'react-router-dom';
import { ipcBridge } from '@/common';
import { parseKnowledgeBaseId } from '@/common/types/ids';
import locale from '@/renderer/services/i18n/locales/zh-CN/messages.json';
import KnowledgeWritebackNotice from './KnowledgeWritebackNotice';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'zh-CN', resources: { 'zh-CN': { translation: { messages: locale } } } });
const kb = parseKnowledgeBaseId('0190f5fe-7c00-7a00-8000-000000000001');
function Location() { return <span data-testid='location'>{useLocation().pathname}</span>; }
afterEach(() => { cleanup(); mock.restore(); });

test('compact notice keeps document details in a popover and opens the exact knowledge base', async () => {
  spyOn(ipcBridge.knowledge.getBase, 'invoke').mockResolvedValue({ name: 'python基础', knowledge_base_id: kb } as never);
  const page = render(<MemoryRouter><I18nextProvider i18n={i18n}>
    <KnowledgeWritebackNotice notice={{ files: [{ kb_id: kb, rel_path: 'python3-loop-statements.md' }], partial: false }} />
    <Location />
  </I18nextProvider></MemoryRouter>);
  const button = page.getByRole('button', { name: '已回写知识库 · 1 篇' });
  expect(button.getAttribute('aria-expanded')).toBe('false');
  expect(page.queryByText('python3-loop-statements.md')).toBeNull();
  await waitFor(() => expect(page.getByText('python基础')).toBeTruthy());
  fireEvent.click(button);
  await waitFor(() => expect(page.getByRole('link', { name: /python3-loop-statements.md/ })).toBeTruthy());
  expect(button.getAttribute('aria-expanded')).toBe('true');
  fireEvent.keyDown(button, { key: 'Escape' });
  expect(button.getAttribute('aria-expanded')).toBe('false');
  expect(document.activeElement).toBe(button);
  fireEvent.click(button);
  fireEvent.click(page.getByRole('link', { name: /python3-loop-statements.md/ }));
  expect(page.getByTestId('location').textContent).toBe(`/knowledge/${kb}`);
});
test('no successful targets produces no notice or metadata request', () => {
  const load = spyOn(ipcBridge.knowledge.getBase, 'invoke');
  const page = render(<MemoryRouter><I18nextProvider i18n={i18n}>
    <KnowledgeWritebackNotice notice={{ files: [], partial: false }} />
  </I18nextProvider></MemoryRouter>);
  expect(page.queryByRole('status')).toBeNull();
  expect(load).not.toHaveBeenCalled();
});
