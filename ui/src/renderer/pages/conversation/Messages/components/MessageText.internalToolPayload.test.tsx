import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import type { IMessageText, TMessage } from '@/common/chat/chatLib';
import { parseConversationId } from '@/common/types/ids';
import MessageText from './MessageText';
import zhMessages from '@/renderer/services/i18n/locales/zh-CN/messages.json';
import enMessages from '@/renderer/services/i18n/locales/en-US/messages.json';
import { MessageListProvider } from '../hooks';

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
  test('displays a proven timeout and cleanup in both languages without rewriting the final report', async () => {
    const raw = 'The command reached its deadline.\n\nUnsuccessful tool attempts in this turn: 1 (including argument checks and command outcomes). Details remain available in the execution steps.\n\nUnsuccessful command attempts in this turn: 1. Each command\'s exit status and output explain the result.';
    const original = { id: 'timeout-footer', type: 'text', position: 'left', turn_id: 'timeout-turn',
      conversation_id: conversationId, created_at: 1, content: { content: raw } } as IMessageText;
    const rows = [{ id: 'timeout', type: 'tool_call', turn_id: original.turn_id, conversation_id: conversationId,
      content: { call_id: 'timeout', name: 'poll_process', status: 'error', output: JSON.stringify({
        state: 'timed_out', success: false, process_id: 'owned', output: { text: '' },
        cleanup: { reaped: true, errors: [], interrupt_attempted: false, terminate_attempted: true, force_kill_attempted: false },
      }) } }] as TMessage[];
    for (const [language, messages, expected] of [
      ['zh-CN', zhMessages, ['1 次命令达到了运行时限', '进程已结束并清理', '1 次调用结果']],
      ['en-US', enMessages, ['1 command(s) reached their time limit', 'cleanup completed', '1 call results']],
    ] as const) {
      const localized = createInstance();
      await localized.use(initReactI18next).init({ lng: language, resources: { [language]: { translation: { messages } } } });
      const view = render(<MemoryRouter><I18nextProvider i18n={localized}><MessageListProvider initialValue={rows}>
        <MessageText message={original} />
      </MessageListProvider></I18nextProvider></MemoryRouter>);
      const visible = () => `${view.container.textContent}${view.container.querySelector('.markdown-shadow')?.shadowRoot?.textContent ?? ''}`;
      await waitFor(() => expected.forEach(text => expect(visible()).toContain(text)));
      expect(visible()).not.toContain('Unsuccessful tool attempts');
      expect(original.content.content).toBe(raw);
      expect(rows[0].type).toBe('tool_call');
      if (rows[0].type === 'tool_call') expect(rows[0].content.status).toBe('error');
      view.unmount();
    }
  });
  test('renders business nonzero and rejected parameters separately in both languages', async () => {
    const raw = 'The diagnostic returned exit 1.\n\nUnsuccessful tool attempts in this turn: 2 (including argument checks and command outcomes). Details remain available in the execution steps.\n\nUnsuccessful command attempts in this turn: 1. Each command\'s exit status and output explain the result.';
    const original = { id:'mixed-footer',type:'text',position:'left',turn_id:'turn-mixed',conversation_id:conversationId,
      created_at:1,content:{content:raw} } as IMessageText;
    const rows = [
      { id:'diagnostic',type:'tool_call',turn_id:original.turn_id,conversation_id:conversationId,content:{
        call_id:'diagnostic',name:'exec_command',status:'error',output:JSON.stringify({
          state:'exited',exit_code:1,signal:null,success:false,process_id:'owned',output:{text:'0 pass, 1 fail'},
          cleanup:{reaped:true,errors:[],interrupt_attempted:false,terminate_attempted:false,force_kill_attempted:false},
        }) } },
      { id:'account',type:'tool_call',turn_id:original.turn_id,conversation_id:conversationId,content:{
        call_id:'account',name:'report_completion',status:'error',output:JSON.stringify({
          status:'not_executed',code:'INVALID_TOOL_ARGUMENTS',tool:'report_completion',
          issues:[{instance_path:'/criteria',error:'stale reference'}],message:'No call in this batch was executed.',
        }) } },
    ] as TMessage[];
    for (const [language,messages,expected] of [
      ['zh-CN',zhMessages,['退出码 1','1 次调用因参数检查未通过而未执行','共计 2 次未成功结果']],
      ['en-US',enMessages,['exit codes: 1','another 1 call(s) did not run','2 unsuccessful results']],
    ] as const) {
      const localized = createInstance();
      await localized.use(initReactI18next).init({lng:language,resources:{[language]:{translation:{messages}}}});
      const view = render(<MemoryRouter><I18nextProvider i18n={localized}><MessageListProvider initialValue={rows}>
        <MessageText message={original} />
      </MessageListProvider></I18nextProvider></MemoryRouter>);
      const visible = () => `${view.container.textContent}${view.container.querySelector('.markdown-shadow')?.shadowRoot?.textContent ?? ''}`;
      await waitFor(() => expected.forEach(text => expect(visible()).toContain(text)));
      expect(visible()).not.toContain('Unsuccessful tool attempts');
      expect(original.content.content).toBe(raw);
      view.unmount();
      const cold = render(<MemoryRouter><I18nextProvider i18n={localized}><MessageListProvider initialValue={[rows[0]]}>
        <MessageText message={original} />
      </MessageListProvider></I18nextProvider></MemoryRouter>);
      const coldText = () => `${cold.container.textContent}${cold.container.querySelector('.markdown-shadow')?.shadowRoot?.textContent ?? ''}`;
      await waitFor(() => expect(coldText()).toContain(language === 'zh-CN' ? '其类别尚未确认' : 'have not been classified'));
      expect(coldText()).toContain(language === 'zh-CN' ? '退出码 1' : 'exit codes: 1');
      expect(coldText()).not.toContain(language === 'zh-CN' ? '参数检查未通过' : 'arguments failed validation');
      expect(original.content.content).toBe(raw);
      cold.unmount();
    }
  });
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
