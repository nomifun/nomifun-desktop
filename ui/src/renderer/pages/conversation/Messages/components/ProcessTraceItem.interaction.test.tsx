import { cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import type { IMessageText, IMessageThinking, IMessageTips, IMessageToolCall, IMessageToolGroup } from '@/common/chat/chatLib';
import { parseConversationId } from '@/common/types/ids';
import ProcessTraceItem from './ProcessTraceItem';
import messagesLocale from '@/renderer/services/i18n/locales/en-US/messages.json';
import chineseMessages from '@/renderer/services/i18n/locales/zh-CN/messages.json';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { messages: messagesLocale } } } });
const conversationId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000051');
const tool = (content: Partial<IMessageToolCall['content']>): IMessageToolCall => ({ id: 'tool', type: 'tool_call',
  conversation_id: conversationId, position: 'left', created_at: 1,
  content: { call_id: 'call', name: 'read_file', status: 'completed', description: 'src/app.ts', ...content } });
const show = (item: Parameters<typeof ProcessTraceItem>[0]['item'], stateOverride?: 'completed') => render(
  <I18nextProvider i18n={i18n}><ProcessTraceItem item={item} stateOverride={stateOverride} /></I18nextProvider>
);
afterEach(cleanup);

describe('public process trace', () => {
  test.each(['running', 'completed', 'error', 'canceled'] as const)(
    '%s file operations retain their path and icon without a body or empty disclosure', status => {
      const item = tool({ status });
      const { container, queryByRole } = show(item);
      expect(container.textContent).toContain('app.ts');
      expect(container.querySelector('.turn-process-trace__row-icon')).not.toBeNull();
      expect(container.querySelector('.turn-process-trace__status')).toBeNull();
      expect(container.querySelector('.turn-process-trace-tool__arrow')).toBeNull();
      expect(container.querySelector('.turn-process-trace-detail')).toBeNull();
      expect(queryByRole('button')).toBeNull();
      expect(item.content.status).toBe(status);
    }
  );

  test.each(['read_file', 'search_files', 'write_file', 'apply_patch', 'write_process_stdin', 'exec_command'])(
    '%s never renders a raw input or output supplied outside the public contract', name => {
      const item = tool({ name, description: name === 'exec_command' ? 'sed -n 1,300p guide.html' : 'src/app.ts' });
      Object.assign(item.content, { args: { path: 'src/app.ts', content: 'PRIVATE WRITE BODY' },
        input: { patch: 'PRIVATE PATCH BODY' }, output: 'PRIVATE FILE CONTENT' });
      const { container, queryByRole } = show(item);
      expect(container.textContent).not.toContain('PRIVATE');
      expect(container.querySelector('pre')).toBeNull();
      expect(queryByRole('button')).toBeNull();
      if (name === 'exec_command') expect(container.textContent).toContain('sed -n 1,300p guide.html');
    }
  );

  test('canonical web actions retain their existing localized target and only a public error expands', async () => {
    const locale = createInstance();
    await locale.use(initReactI18next).init({ lng: 'zh-CN', resources: { 'zh-CN': { translation: { messages: chineseMessages } } } });
    const name = 'platform__web_research_web_research_fetch__30fb0227939eb7c80164';
    const item = tool({ name, capability_id: 'web.research', action_id: 'web.research/fetch',
      status: 'error', description: 'pvp.qq.com', error: 'The page could not be read.' });
    const view = render(<I18nextProvider i18n={locale}><ProcessTraceItem item={item} /></I18nextProvider>);
    const header = view.getByRole('button');
    expect(header.textContent).toContain('读取网页 · pvp.qq.com');
    expect(header.textContent).not.toContain('platform__');
    fireEvent.click(header);
    expect(view.container.textContent).toContain('The page could not be read.');
    expect(view.container.textContent).not.toContain(name);
    expect(view.container.textContent).not.toContain('web.research/fetch');
    expect(view.container.textContent).not.toContain('调用标识');
  });

  test.each([
    [{ command_timed_out: true }, 'reached its time limit; process cleanup completed'],
    [{ command_not_started: true }, 'did not start; command was not run'],
    [{ command_exit_code: 1, non_fatal_failure: true }, 'ended with exit code 1'],
  ])('typed process outcome preserves the existing command row', (metadata, expected) => {
    const item = tool({ name: 'exec_command', status: 'error', description: 'bun test', ...metadata });
    const { container, queryByRole } = show(item);
    expect(container.textContent).toContain('bun test');
    expect(container.textContent).toContain(expected);
    expect(queryByRole('button')).toBeNull();
  });

  test.each([
    ['prior_call_failed', 'Skipped'], ['arguments_rejected', 'Arguments did not pass validation'],
    ['input_superseded', 'Did not run'],
  ] as const)('pre-dispatch %s preserves the neutral row', (runtime_hold, label) => {
    const { container, queryByRole } = show(tool({ name: 'write_file', status: 'error', runtime_hold }), 'completed');
    expect(container.textContent).toContain(label);
    expect(container.querySelector('.turn-process-trace__row--failed')).toBeNull();
    expect(queryByRole('button')).toBeNull();
  });

  test('a rejected model proposal cannot be expanded into its embedded file body', () => {
    const item: IMessageText = { id: 'raw-tool-payload', type: 'text', conversation_id: conversationId,
      position: 'left', created_at: 1,
      content: { content: '<tool_call>\n<function=write_file>\n<parameter=content>\n<!DOCTYPE html>' } };
    const { container, queryByRole } = show(item);
    expect(container.textContent).toContain('Invalid tool-call format');
    expect(container.querySelector('details')).toBeNull();
    expect(container.querySelector('pre')).toBeNull();
    expect(container.textContent).not.toContain('<!DOCTYPE html>');
    expect(queryByRole('button')).toBeNull();
  });

  test('whitespace-only assistant fragments leave no blank process block', () => {
    const item: IMessageText = { id: 'empty-fragment', type: 'text', conversation_id: conversationId,
      position: 'left', created_at: 1, content: { content: '\n \n\n' } };
    expect(show(item).container.childElementCount).toBe(0);
  });

  test.each(['[Private reasoning omitted]', '[Private reasoning omitted from replay]'])(
    'does not render private reasoning placeholder %s', content => {
      const item: IMessageThinking = { id: 'private-placeholder', type: 'thinking',
        conversation_id: conversationId, created_at: 1, content: { content, status: 'done' } };
      expect(show(item).container.childElementCount).toBe(0);
    }
  );

  test('context compression remains a lightweight existing system row', () => {
    const item: IMessageTips = { id: 'compaction', type: 'tips', conversation_id: conversationId,
      created_at: 1, content: { type: 'success', content: 'Context compaction completed' } };
    const { container, queryByRole } = show(item);
    expect(container.textContent).toBe(messagesLocale.processReceipt.contextCompressed);
    expect(container.querySelector('.turn-process-trace__row-icon')).not.toBeNull();
    expect(queryByRole('button')).toBeNull();
  });

  test('thinking disclosure behavior is unchanged', () => {
    const item: IMessageThinking = { id: 'thinking', type: 'thinking', conversation_id: conversationId,
      position: 'left', created_at: 1, content: { content: 'Earlier result', status: 'thinking' } };
    const view = (completed = false) => <MemoryRouter><I18nextProvider i18n={i18n}>
      <ProcessTraceItem item={item} stateOverride={completed ? 'completed' : undefined} />
    </I18nextProvider></MemoryRouter>;
    const { container, rerender } = render(view());
    expect(container.querySelector('[data-thinking-process-header]')?.getAttribute('aria-expanded')).toBe('true');
    rerender(view(true));
    expect(container.querySelector('[data-thinking-process-header]')?.getAttribute('aria-expanded')).toBe('false');
    fireEvent.click(container.querySelector<HTMLButtonElement>('[data-thinking-process-header]')!);
    expect(container.querySelector('[data-thinking-process-header]')?.getAttribute('aria-expanded')).toBe('true');
  });

  test('file group rows preserve their existing paths without a second disclosure', () => {
    const item: IMessageToolGroup = { id: 'edited-files', type: 'tool_group', conversation_id: conversationId,
      position: 'left', created_at: 2, content: ['src/a.ts', 'src/b.ts', 'src/c.ts'].map((path, index) => ({
        call_id: `edit-${index}`, name: 'write_file', status: 'Success' as const, description: path,
      })) };
    const { container, queryByRole } = show(item);
    expect(container.querySelectorAll('.turn-process-trace > .turn-process-trace-tool')).toHaveLength(3);
    expect(container.textContent).toContain('Write file · a.ts');
    expect(container.textContent).toContain('Write file · c.ts');
    expect(queryByRole('button')).toBeNull();
  });

  test('an unconfirmed result retains its existing inline status without an empty detail panel', () => {
    const item = tool({ name: 'browser_navigate', status: 'running', description: 'example.com',
      timeline: { through_seq: 20, segments: [{ segment_id: 'tool-start', group_id: 'process-start', first_seq: 3,
        last_seq: 20, kind: 'tool', call_id: 'call', state: 'interrupted' }] } });
    const { container, queryByRole } = show(item, 'completed');
    expect(container.textContent).toContain(messagesLocale.toolResultUnconfirmed);
    expect(container.textContent).toContain('example.com');
    expect(container.querySelector('.turn-process-trace__row--failed')).toBeNull();
    expect(queryByRole('button')).toBeNull();
    expect(item.content.status).toBe('running');
  });
});
