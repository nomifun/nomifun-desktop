import { expect, test } from 'bun:test';
import { renderToStaticMarkup } from 'react-dom/server';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import conversation from '@/renderer/services/i18n/locales/zh-CN/conversation.json';
import messages from '@/renderer/services/i18n/locales/zh-CN/messages.json';
import type { TChatConversation } from '@/common/config/storage';
import ConversationHoverCard from './ConversationHoverCard';

test('the session status distinguishes a canonical pause without the retired banner', async () => {
  const i18n = createInstance();
  await i18n.init({ lng: 'zh-CN', resources: { 'zh-CN': { translation: { conversation, messages } } } });
  const session = { id: '0190f5fe-7c00-7a00-8000-000000000081', name: 'paused task',
    status: 'running', extra: { execution_phase: 'paused' }, runtime: { state: 'idle' } } as TChatConversation;
  const html = renderToStaticMarkup(<I18nextProvider i18n={i18n}><ConversationHoverCard conversation={session} /></I18nextProvider>);
  expect(html).toContain('已暂停');
  expect(html).not.toContain('>空闲<');
});
