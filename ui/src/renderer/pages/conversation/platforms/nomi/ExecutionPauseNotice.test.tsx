import { expect, test } from 'bun:test';
import { renderToStaticMarkup } from 'react-dom/server';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { parseMessageId } from '@/common/types/ids';
import { ExecutionPauseNotice } from './ExecutionPauseNotice';
import translations from '@/renderer/services/i18n/locales/zh-CN/conversation.json';
import ConversationHoverCard from '../../components/ConversationHoverCard';
import type { TChatConversation } from '@/common/config/storage';

test('paused UI gives the provider cause, preserves completed work and keeps new messages blocked', async () => {
  const i18n = createInstance();
  await i18n.init({ lng: 'zh-CN', resources: { 'zh-CN': { translation: { conversation: translations } } } });
  const pause = { turnId: parseMessageId('0190f5fe-7c00-7a00-8000-000000000081'),
    reason: 'EXECUTION_MODEL_PROVIDER_UNAVAILABLE', cleanupProven: true, pausedAt: 123 };
  const render = (cleanupProven: boolean) => renderToStaticMarkup(
    <I18nextProvider i18n={i18n}><ExecutionPauseNotice pause={{ ...pause, cleanupProven }} /></I18nextProvider>
  );
  expect(render(true)).toContain('role="status"');
  expect(render(true)).toContain('模型服务暂时不可用');
  expect(render(true)).toContain('已完成的操作会保留');
  expect(render(true)).toContain('暂不接受新消息');
  expect(render(false)).toContain('资源清理状态尚未确认');
  expect(render(false)).not.toContain('模型服务暂时不可用');
});

test('the session hover card distinguishes a paused task from an idle stream', async () => {
  const i18n = createInstance();
  await i18n.init({ lng: 'zh-CN', resources: { 'zh-CN': { translation: { conversation: translations } } } });
  const conversation = { id: '0190f5fe-7c00-7a00-8000-000000000081', name: 'paused task',
    status: 'running', extra: { execution_phase: 'paused' }, runtime: { state: 'idle' } } as TChatConversation;
  const html = renderToStaticMarkup(<I18nextProvider i18n={i18n}><ConversationHoverCard conversation={conversation} /></I18nextProvider>);
  expect(html).toContain('执行已暂停');
  expect(html).not.toContain('>空闲<');
});
