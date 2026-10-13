import '../../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter, useLocation } from 'react-router-dom';
import { ipcBridge } from '@/common';
import type { TChatConversation, TProviderWithModel } from '@/common/config/storage';
import type { IMessageSearchItem } from '@/common/types/conversationSearch';
import { parseConversationId, parseMessageId, parseProviderId } from '@/common/types/ids';
import * as Theme from '@/renderer/hooks/context/ThemeContext';
import appLocale from '@/renderer/services/i18n/locales/en-US/appHosting.json';
import conversationLocale from '@/renderer/services/i18n/locales/en-US/conversation.json';
import ConversationSearchPopover from './ConversationSearchPopover';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { appHosting: appLocale, conversation: conversationLocale } } }, interpolation: { escapeValue: false } });
const APP = 'reminder-app';
const RECENT_SEARCH_KEY = 'conversation.historySearch.recentKeywords';
const model: TProviderWithModel = {
  id: parseProviderId('0190f5fe-7c00-7a00-8000-000000000603'), name: 'Configured provider',
  platform: 'stepfun', base_url: 'https://example.invalid/v1', auth_scheme: 'bearer',
  has_credentials: true, enabled: true, use_model: 'chat-model',
};
const conversation: TChatConversation = {
  id: parseConversationId('0190f5fe-7c00-7a00-8000-000000000601'), name: 'Ordinary session',
  type: 'nomi', session_purpose: 'conversation', created_at: 1, modified_at: 2, model,
  extra: { workspace: 'C:/NomiFun/apps/reminder-app', official_template_key: 'app.developer' } as TChatConversation['extra'],
};
const ordinaryResult: IMessageSearchItem = {
  conversation, message_id: parseMessageId('0190f5fe-7c00-7a00-8000-000000000611'),
  message_type: 'text', message_created_at: 1000, preview_text: 'A reminder in the ordinary conversation',
};
const ownedResult: IMessageSearchItem = {
  ...ordinaryResult,
  conversation: { ...conversation, id: parseConversationId('0190f5fe-7c00-7a00-8000-000000000602'), name: 'Application session', app_project_id: APP },
  message_id: parseMessageId('0190f5fe-7c00-7a00-8000-000000000612'),
  preview_text: 'A reminder in the application project',
};

beforeEach(() => {
  localStorage.removeItem(RECENT_SEARCH_KEY);
  spyOn(Theme, 'useThemeContext').mockReturnValue({
    theme: 'light', colorScheme: 'default', fontScale: 1,
    setTheme: async () => {}, setColorScheme: async () => {}, setFontScale: async () => {},
  });
});
afterEach(() => { cleanup(); mock.restore(); localStorage.removeItem(RECENT_SEARCH_KEY); });

function Location() {
  const location = useLocation();
  return <output data-testid='route'>{JSON.stringify({ path: location.pathname + location.search, state: location.state })}</output>;
}

async function fixture() {
  const search = spyOn(ipcBridge.database.searchConversationMessages, 'invoke').mockResolvedValue({
    items: [ownedResult, ordinaryResult], total: 2, has_more: false,
  });
  const selected = mock(() => {});
  const sessionClicked = mock(() => {});
  const view = render(<MemoryRouter initialEntries={['/guid']}><I18nextProvider i18n={i18n}>
    <Location /><ConversationSearchPopover onConversationSelect={selected} onSessionClick={sessionClicked}
      renderTrigger={({ onClick, isActive }) => <button onClick={onClick} aria-expanded={isActive}>Open history search</button>} />
  </I18nextProvider></MemoryRouter>);
  fireEvent.click(view.getByRole('button', { name: 'Open history search' }));
  const input = await view.findByPlaceholderText(conversationLocale.historySearch.placeholder);
  fireEvent.change(input, { target: { value: 'reminder' } });
  const appButton = await view.findByRole('button', { name: /Application session/ });
  const ordinaryButton = view.getByRole('button', { name: /Ordinary session/ });
  return { view, search, selected, sessionClicked, appButton, ordinaryButton };
}

test('search marks App project results and opens their owned workspace with the exact matched message', async () => {
  const f = await fixture();
  expect(within(f.appButton).getByText(appLocale.project.searchLabel)).toBeTruthy();
  expect(within(f.ordinaryButton).queryByText(appLocale.project.searchLabel)).toBeNull();
  expect(f.search.mock.calls[0][0]).toEqual({ keyword: 'reminder', page: 0, page_size: 20 });
  fireEvent.click(f.appButton);
  await waitFor(() => expect(JSON.parse(f.view.getByTestId('route').textContent!)).toEqual({
    path: `/apps/${APP}?session=${ownedResult.conversation.id}`,
    state: { targetMessageId: ownedResult.message_id, fromConversationSearch: true },
  }));
  await waitFor(() => expect(f.sessionClicked).toHaveBeenCalledTimes(1));
  expect(f.selected).toHaveBeenCalledTimes(1);
  expect(f.view.getByRole('button', { name: 'Open history search' }).getAttribute('aria-expanded')).toBe('false');
});

test('search opens same-directory ordinary Session results in the conversation surface without an App ownership label', async () => {
  const f = await fixture();
  expect(within(f.ordinaryButton).queryByText(appLocale.project.searchLabel)).toBeNull();
  fireEvent.click(f.ordinaryButton);
  await waitFor(() => expect(JSON.parse(f.view.getByTestId('route').textContent!)).toEqual({
    path: `/conversation/${ordinaryResult.conversation.id}`,
    state: { targetMessageId: ordinaryResult.message_id, fromConversationSearch: true },
  }));
  await waitFor(() => expect(f.sessionClicked).toHaveBeenCalledTimes(1));
  expect(f.selected).toHaveBeenCalledTimes(1);
});
