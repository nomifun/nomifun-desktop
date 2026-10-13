import '../../../../test/setup-dom.ts';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter, Route, Routes, useLocation, useNavigate } from 'react-router-dom';
import { SWRConfig } from 'swr';
import { ipcBridge } from '@/common';
import type { TChatConversation, TProviderWithModel } from '@/common/config/storage';
import { parseConversationId, parseMessageId, parseProviderId } from '@/common/types/ids';
import * as AutoTitle from '@/renderer/hooks/chat/useAutoTitle';
import * as ChatModule from './components/ChatConversation';
import conversationLocale from '@/renderer/services/i18n/locales/en-US/conversation.json';
import ChatConversationIndex from './index';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { conversation: conversationLocale } } }, interpolation: { escapeValue: false } });
const SESSION = parseConversationId('0190f5fe-7c00-7a00-8000-000000000501');
const MESSAGE = parseMessageId('0190f5fe-7c00-7a00-8000-000000000502');
const APP = 'reminder-app';
const model: TProviderWithModel = {
  id: parseProviderId('0190f5fe-7c00-7a00-8000-000000000503'), name: 'Configured provider',
  platform: 'stepfun', base_url: 'https://example.invalid/v1', auth_scheme: 'bearer',
  has_credentials: true, enabled: true, use_model: 'chat-model',
};
const ordinary: TChatConversation = {
  id: SESSION, name: 'Ordinary application development', type: 'nomi', session_purpose: 'conversation',
  created_at: 1, modified_at: 2, model,
  extra: { workspace: 'C:/NomiFun/apps/reminder-app', official_template_key: 'app.developer' } as TChatConversation['extra'],
};
const syncTitle = mock(async () => {});
let renderedConversations: Array<TChatConversation | undefined>;

beforeEach(() => {
  renderedConversations = [];
  syncTitle.mockClear();
  spyOn(ChatModule, 'default').mockImplementation(({ conversation }) => {
    renderedConversations.push(conversation);
    return <div data-testid='ordinary-chat'>{conversation?.id}</div>;
  });
  spyOn(AutoTitle, 'useAutoTitle').mockReturnValue({ syncTitleFromHistory: syncTitle, checkAndUpdateTitle: syncTitle });
  spyOn(ipcBridge.conversation.listChanged, 'on').mockReturnValue(() => {});
});
afterEach(() => { cleanup(); mock.restore(); });

function Location() {
  const location = useLocation();
  const navigate = useNavigate();
  return <>
    <output data-testid='route'>{JSON.stringify({ path: location.pathname + location.search, state: location.state })}</output>
    <button onClick={() => { void navigate(-1); }}>Go back</button>
  </>;
}

function fixture(state?: unknown) {
  return render(<MemoryRouter initialEntries={['/apps', { pathname: `/conversation/${SESSION}`, state }]} initialIndex={1}>
    <I18nextProvider i18n={i18n}><SWRConfig value={{ provider: () => new Map(), dedupingInterval: 0, shouldRetryOnError: false }}>
      <Location /><Routes>
        <Route path='/conversation/:id' element={<ChatConversationIndex />} />
        <Route path='/apps/:id' element={<div>Application workspace</div>} />
        <Route path='/apps' element={<div>Applications</div>} />
      </Routes>
    </SWRConfig></I18nextProvider>
  </MemoryRouter>);
}

test('an owned Session deep link redirects to its App without rendering ordinary chat and preserves the message target', async () => {
  const owned = { ...ordinary, app_project_id: APP, name: conversationLocale.welcome.newConversation };
  let complete!: (conversation: TChatConversation) => void;
  const get = spyOn(ipcBridge.conversation.get, 'invoke').mockImplementation(() => new Promise(resolve => { complete = resolve; }));
  const state = { targetMessageId: MESSAGE, fromConversationSearch: true };
  const view = fixture(state);
  await waitFor(() => expect(get).toHaveBeenCalledTimes(1));
  expect(view.getByTestId('message-list-skeleton')).toBeTruthy();
  expect(renderedConversations).toHaveLength(0);
  await act(async () => { complete(owned); });
  await waitFor(() => expect(JSON.parse(view.getByTestId('route').textContent!)).toEqual({
    path: `/apps/${APP}?session=${SESSION}`, state,
  }));
  expect(get.mock.calls[0][0]).toEqual({ conversation_id: SESSION });
  expect(view.queryByTestId('ordinary-chat')).toBeNull();
  expect(renderedConversations).toHaveLength(0);
  expect(syncTitle).not.toHaveBeenCalled();

  fireEvent.click(view.getByRole('button', { name: 'Go back' }));
  await waitFor(() => expect(JSON.parse(view.getByTestId('route').textContent!).path).toBe('/apps'));
  expect(get).toHaveBeenCalledTimes(1);
});

test('a same-directory ordinary Session retains its conversation route and chat despite using the App developer Agent', async () => {
  spyOn(ipcBridge.conversation.get, 'invoke').mockResolvedValue(ordinary);
  const state = { targetMessageId: MESSAGE, fromConversationSearch: true };
  const view = fixture(state);
  await view.findByTestId('ordinary-chat');
  expect(JSON.parse(view.getByTestId('route').textContent!)).toEqual({ path: `/conversation/${SESSION}`, state });
  expect(renderedConversations.at(-1)).toEqual(ordinary);
  expect(syncTitle).not.toHaveBeenCalled();
});
