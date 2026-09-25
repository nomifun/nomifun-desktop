import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import type { IMessageText } from '@/common/chat/chatLib';
import { parseConversationId } from '@/common/types/ids';
import MessageText from './MessageText';

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
