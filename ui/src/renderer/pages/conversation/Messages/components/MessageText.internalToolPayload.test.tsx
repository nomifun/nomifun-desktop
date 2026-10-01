import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import type { IMessageText } from '@/common/chat/chatLib';
import { parseConversationId } from '@/common/types/ids';
import MessageText from './MessageText';
import zhMessages from '@/renderer/services/i18n/locales/zh-CN/messages.json';

const i18n = createInstance();
await i18n.use(initReactI18next).init({
  lng: 'en-US',
  resources: { 'en-US': { translation: {
    messages: {
      internalToolPayloadOmitted: 'Unparsed tool-call content was hidden; no displayable reply was produced.',
    },
  } } },
});
const conversationId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000051');

afterEach(cleanup);

const renderMessage = (content: string, position: IMessageText['position']) => {
  const message: IMessageText = {
    id: `tool-text-${position}`, type: 'text', conversation_id: conversationId,
    position, created_at: 1, content: { content },
  };
  return render(
    <MemoryRouter><I18nextProvider i18n={i18n}><MessageText message={message} /></I18nextProvider></MemoryRouter>
  );
};

describe('MessageText internal tool payload display', () => {
  test('localizes the exact runtime count footer without changing the canonical message', async () => {
    const localized = createInstance();
    await localized.use(initReactI18next).init({ lng:'zh-CN', resources:{'zh-CN':{translation:{messages:zhMessages}}} });
    const raw = '诊断退出码为 1。\n\nUnsuccessful tool attempts in this turn: 1 (including argument checks and command outcomes). Details remain available in the execution steps.\n\nUnsuccessful command attempts in this turn: 1. Each command\'s exit status and output explain the result.';
    const original = { id:'localized-footer',type:'text',position:'left',conversation_id:conversationId,created_at:1,
      content:{content:raw} } as IMessageText;
    const { container } = render(<MemoryRouter><I18nextProvider i18n={localized}><MessageText message={original} /></I18nextProvider></MemoryRouter>);
    const visible = () => `${container.textContent}${container.querySelector('.markdown-shadow')?.shadowRoot?.textContent ?? ''}`;
    await waitFor(() => expect(visible()).toContain('调用 1 次，命令 1 次'));
    expect(visible()).not.toContain('Unsuccessful tool attempts');
    expect(visible()).toContain('诊断退出码为 1');
    expect(original.content.content).toBe(raw);
  });
  test('ordinary final text retains its copy action', () => {
    const { container } = renderMessage('The file is ready.', 'left');
    expect(container.querySelector('[data-testid="message-copy-action"]')).not.toBeNull();
  });
  test('renders malformed assistant output as a closed diagnostic instead of a successful reply', async () => {
    const raw = '<tool_call>\n<function=write_file>\n<parameter=content>\n<!DOCTYPE html>\n<style>body{color:red}</style>';
    const { container } = renderMessage(raw, 'left');
    expect(container.textContent).toContain('Invalid tool-call format');
    expect(container.textContent).not.toContain('<!DOCTYPE html>');
    expect(container.querySelector('pre')).toBeNull();
    expect(container.querySelector('[data-testid="message-copy-action"]')).toBeNull();
    const details = container.querySelector('details')!;
    await act(async () => {
      details.open = true;
      fireEvent(details, new Event('toggle'));
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    expect(container.querySelector('pre')?.textContent).toBe(raw);
  });

  test('never alters text supplied by the user', () => {
    const raw = '<tool_call>\n<function=write_file>\n<parameter=content>\nexample';
    const { container } = renderMessage(raw, 'right');
    expect(container.textContent).toContain('<tool_call>');
    expect(container.textContent).toContain('example');
  });
});
