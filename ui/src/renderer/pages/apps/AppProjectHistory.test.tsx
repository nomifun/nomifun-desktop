import '../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, expect, mock, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import type { TChatConversation } from '@/common/config/storage';
import { parseConversationId } from '@/common/types/ids';
import locale from '../../services/i18n/locales/en-US/appHosting.json';
import AppProjectHistory from './AppProjectHistory';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { appHosting: locale } } }, interpolation: { escapeValue: false } });
const conversation = { id: parseConversationId('0190f5fe-7c00-7a00-8000-000000000301'), type: 'nomi', name: 'First iteration', created_at: 1, modified_at: 2,
  extra: { workspace: 'C:/apps/project' } } as TChatConversation;
const wrapper = ({ children }: { children: React.ReactNode }) => <I18nextProvider i18n={i18n}>{children}</I18nextProvider>;
afterEach(cleanup);

test('project history selects an editable Session and views another Agent in the same application', () => {
  const select = mock(() => {}), open = mock(() => {}), create = mock(() => {}), more = mock(() => {});
  const foreignId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000302');
  const view = render(<AppProjectHistory items={[{ conversation, editable_in_app: true, archived: false }, { conversation: { ...conversation, id: foreignId, name: 'My own Agent' }, editable_in_app: false, archived: true }]}
    selectedSessionId={conversation.id} total={5} loading={false} hasMore onSelect={select} onViewSession={open} onNewSession={create} onLoadMore={more} onRefresh={() => {}} />, { wrapper });
  const details = view.container.querySelector('details')!;
  fireEvent.click(view.getByText(locale.project.historyHint));
  details.open = true;
  expect(view.getByText(locale.project.archived)).toBeTruthy();
  expect(view.getByText(locale.project.readOnly)).toBeTruthy();
  fireEvent.click(view.getByRole('button', { name: /My own Agent/ }));
  expect(open).toHaveBeenCalledWith(foreignId); expect(select).not.toHaveBeenCalled(); expect(details.open).toBe(false);
  details.open = true;
  fireEvent.click(view.getByRole('button', { name: /First iteration/ }));
  expect(select).toHaveBeenCalledWith(conversation.id);
  fireEvent.click(view.getByRole('button', { name: locale.project.newSession })); expect(create).toHaveBeenCalledTimes(1);
  fireEvent.click(view.getByRole('button', { name: locale.project.loadMore })); expect(more).toHaveBeenCalledTimes(1);
});

test('history read failure retains existing items and keyboard escape returns focus to the selector', () => {
  const retry = mock(() => {});
  const view = render(<AppProjectHistory items={[{ conversation, editable_in_app: true, archived: false }]} selectedSessionId={conversation.id} total={1} loading={false}
    error='History connection unavailable' hasMore={false} newSessionDisabled onSelect={() => {}} onViewSession={() => {}} onNewSession={() => {}} onLoadMore={() => {}} onRefresh={retry} />, { wrapper });
  expect(view.getByRole('button', { name: /First iteration/ })).toBeTruthy();
  expect((view.getByRole('button', { name: locale.project.newSession }) as HTMLButtonElement).disabled).toBe(true);
  expect(view.getByText('History connection unavailable')).toBeTruthy();
  fireEvent.click(view.getByRole('button', { name: locale.project.refresh })); expect(retry).toHaveBeenCalledTimes(1);
  const details = view.container.querySelector('details')!; details.open = true;
  fireEvent.keyDown(view.getByRole('button', { name: /First iteration/ }), { key: 'Escape' });
  expect(details.open).toBe(false); expect(document.activeElement).toBe(details.querySelector('summary'));
});
