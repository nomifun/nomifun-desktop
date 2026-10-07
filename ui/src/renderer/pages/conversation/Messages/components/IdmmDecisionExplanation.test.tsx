import { afterEach, expect, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import type { IMessageText, IMessageTips } from '@/common/chat/chatLib';
import { configService } from '@/common/config/configService';
import { parseConversationId, parseMessageId } from '@/common/types/ids';
import type { IdmmDecisionExplanation as Decision } from '@/common/types/idmm';
import { CHAT_MESSAGE_JUMP_EVENT } from '@/renderer/utils/chat/chatMinimapEvents';
import { emitter } from '@/renderer/utils/emitter';
import * as clipboard from '@/renderer/utils/ui/clipboard';
import { ConversationProvider } from '@/renderer/hooks/context/ConversationContext';
import { MessageListProvider } from '../hooks';
import MessageText from './MessageText';
import MessageTips from './MessageTips';
import zhIdmm from '@/renderer/services/i18n/locales/zh-CN/idmm.json';
import enIdmm from '@/renderer/services/i18n/locales/en-US/idmm.json';

const conversationId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000051');
const decision: Decision = { intervention_id: '0190f5fe-7c00-7a00-8000-000000000081', source: 'bypass_model',
  reason_code: 'bypass_model_decision', rationale: 'A bounded cache fits the request.',
  model: { provider_id: '0190f5fe-7c00-7a00-8000-000000000082', model: 'step-3.5' },
  question: { message_id: '0190f5fe-7c00-7a00-8000-000000000083', sequence: 5, fingerprint: 'a'.repeat(64) } };
const message: IMessageText = { id: 'automatic', type: 'text', position: 'right', conversation_id: conversationId,
  created_at: 1, content: { content: 'Use LRU and a 30 minute TTL.', idmm_decision: decision } };
const localized = async (language: 'zh-CN' | 'en-US') => {
  const i18n = createInstance();
  await i18n.init({ lng: language, resources: { [language]: { translation: { idmm: language === 'zh-CN' ? zhIdmm : enIdmm } } } });
  return i18n;
};
afterEach(() => { cleanup(); configService.setLocal('chat.idmm.showDecisionBasis', undefined); });

test('automatic reply keeps its body and copy content while its actual source and collapsible basis sit outside the bubble', async () => {
  const copied = spyOn(clipboard, 'copyText').mockResolvedValue();
  try {
    const view = render(<MemoryRouter><I18nextProvider i18n={await localized('en-US')}><MessageText message={message} /></I18nextProvider></MemoryRouter>);
    const body = view.getByTestId('message-text-content');
    const explanation = view.getByTestId('idmm-decision-explanation');
    expect(body.textContent).toBe(message.content.content);
    expect(body.contains(explanation)).toBe(false);
    expect(explanation.textContent).toContain('step-3.5 decision');
    expect(explanation.textContent).toContain('Smart Decision · step-3.5 decision');
    expect(view.getByTestId('idmm-decision-basis').hidden).toBe(true);
    fireEvent.click(view.getByRole('button', { name: 'Decision basis' }));
    expect(view.getByTestId('idmm-decision-basis').hidden).toBe(false);
    await act(async () => { fireEvent.click(view.getByTestId('message-copy-action')); });
    expect(copied).toHaveBeenCalledWith(message.content.content);
    let target: unknown;
    const listener = (event: Event) => { target = (event as CustomEvent).detail; };
    window.addEventListener(CHAT_MESSAGE_JUMP_EVENT, listener);
    fireEvent.click(view.getByRole('button', { name: 'View original question' }));
    window.removeEventListener(CHAT_MESSAGE_JUMP_EVENT, listener);
    expect(target).toMatchObject({ conversation_id: conversationId, messageId: decision.question!.message_id, loadOlder: true });
  } finally { copied.mockRestore(); }
});

test('display preference expands existing basis without changing model data; unannotated replies do not guess provenance', async () => {
  configService.setLocal('chat.idmm.showDecisionBasis', true);
  const i18n = await localized('zh-CN');
  const view = render(<MemoryRouter><I18nextProvider i18n={i18n}><MessageText message={message} /></I18nextProvider></MemoryRouter>);
  expect(view.getByTestId('idmm-decision-basis').hidden).toBe(false);
  expect(view.getByText('step-3.5 决策')).toBeTruthy();
  view.rerender(<MemoryRouter><I18nextProvider i18n={i18n}><MessageText message={{ ...message, content: { content: '2' } }} /></I18nextProvider></MemoryRouter>);
  expect(view.queryByTestId('idmm-decision-explanation')).toBeNull();
  expect(message.content.idmm_decision).toEqual(decision);
});

test('waiting and failed outcomes remain historical notes; add input only focuses the owning composer', async () => {
  const i18n = await localized('en-US');
  const notice: IMessageTips = { ...message, type: 'tips', position: 'center', content: {
    content: '', type: 'warning', idmm_notice: { decision, status: 'waiting_for_human', created_at: 1234 },
  } };
  const view = render(<MemoryRouter><I18nextProvider i18n={i18n}><MessageTips message={notice} /></I18nextProvider></MemoryRouter>);
  expect(view.getByTestId('idmm-decision-notice').textContent).toContain('Human input was needed here');
  expect(view.container.textContent).not.toContain('paused');
  let focused: unknown;
  const listener = (id: unknown) => { focused = id; };
  emitter.on('sendbox.focus', listener);
  fireEvent.click(view.getByRole('button', { name: 'Add input' }));
  emitter.off('sendbox.focus', listener);
  expect(focused).toBe(conversationId);
  view.rerender(<MemoryRouter><I18nextProvider i18n={i18n}><MessageTips message={{ ...notice, content: {
    ...notice.content, idmm_notice: { ...notice.content.idmm_notice!, status: 'failed' },
  } }} /></I18nextProvider></MemoryRouter>);
  expect(view.container.textContent).toContain('Automatic decision did not complete here');
  expect(view.queryByTestId('message-error-retry')).toBeNull();
});

test('the latest automatic input cannot be edited as human text while a real human input retains editing', async () => {
  const latest: IMessageText = { ...message, message_id: parseMessageId('0190f5fe-7c00-7a00-8000-000000000084') };
  const i18n = await localized('en-US');
  const content = (row: IMessageText) => <MemoryRouter><I18nextProvider i18n={i18n}>
    <ConversationProvider value={{ conversation_id: conversationId, type: 'nomi', isProcessing: false }}>
      <MessageListProvider initialValue={[row]}><MessageText message={row} /></MessageListProvider>
    </ConversationProvider>
  </I18nextProvider></MemoryRouter>;
  const automatic = render(content(latest));
  expect(automatic.queryByTestId('message-edit-action')).toBeNull();
  automatic.unmount();
  const human = render(content({ ...latest, content: { content: 'A human request' } }));
  expect(human.queryByTestId('message-edit-action')).not.toBeNull();
});

test('English fixed rule and recovery bases follow explicit reason codes while model reasons stay verbatim', async () => {
  configService.setLocal('chat.idmm.showDecisionBasis', true);
  const i18n = await localized('en-US');
  const view = (value: Decision) => <MemoryRouter><I18nextProvider i18n={i18n}>
    <MessageText message={{ ...message, content: { ...message.content, idmm_decision: value } }} />
  </I18nextProvider></MemoryRouter>;
  const rule: Decision = { ...decision, source: 'rule', model: undefined,
    reason_code: 'rule_selected_recommended_option', rationale: '候选项包含推荐的安全选项。' };
  const page = render(view(rule));
  expect(page.getByTestId('idmm-decision-explanation').textContent).toContain('Smart Decision · Rule guard');
  expect(page.getByTestId('idmm-decision-basis').textContent).toBe('Chose the recommended safe option.');
  page.rerender(view({ ...rule, reason_code: 'rule_selected_first_safe_option' }));
  expect(page.getByTestId('idmm-decision-basis').textContent).toBe('Chose the first option that passed safety checks.');
  page.rerender(view({ ...rule, source: 'recovery', reason_code: 'provider_fault_detected' }));
  expect(page.getByTestId('idmm-decision-explanation').textContent).toContain('Smart Decision · Automatic recovery');
  expect(page.getByTestId('idmm-decision-basis').textContent).toContain('retryable provider fault');
  page.rerender(view({ ...decision, reason_code: 'bypass_model_failed', rationale: '旁路模型未返回有效决策。' }));
  expect(page.getByTestId('idmm-decision-basis').textContent).toBe('The bypass model returned no valid decision; no reply was sent.');
  for (const reason_code of ['bypass_model_decision', 'bypass_model_halted', 'future_reason', 'constructor']) {
    page.rerender(view({ ...decision, reason_code, rationale: '保留模型给出的具体原因。' }));
    expect(page.getByTestId('idmm-decision-basis').textContent).toBe('保留模型给出的具体原因。');
  }
  expect(page.getByTestId('message-text-content').textContent).toBe(message.content.content);
});
