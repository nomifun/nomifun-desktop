import { cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import type { IMessageText, IMessageThinking, IMessageToolCall, IMessageToolGroup } from '@/common/chat/chatLib';
import { parseConversationId } from '@/common/types/ids';
import ProcessTraceItem from './ProcessTraceItem';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: {} } } });
const conversationId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000051');

afterEach(cleanup);

describe('replayed process trace', () => {
  test('whitespace-only assistant fragments leave no blank process block', () => {
    const item: IMessageText = {
      id: 'empty-fragment', type: 'text', conversation_id: conversationId,
      position: 'left', created_at: 1, content: { content: '\n \n\n' },
    };
    const { container } = render(<I18nextProvider i18n={i18n}><ProcessTraceItem item={item} /></I18nextProvider>);
    expect(container.childElementCount).toBe(0);
  });

  test('intermediate assistant prose is replaced by one neutral result-preparation status', () => {
    const note = 'Repeating progress should remain inspectable. '.repeat(12);
    const item: IMessageText = {
      id: 'long-progress', type: 'text', conversation_id: conversationId,
      position: 'left', created_at: 1, content: { content: note },
    };
    const { container } = render(
      <I18nextProvider i18n={i18n}><ProcessTraceItem item={item} /></I18nextProvider>
    );
    expect(container.textContent).toContain('Prepared the result');
    expect(container.textContent).not.toContain('Repeating progress');
    expect(container.querySelector('button')).toBeNull();
  });

  test('private thinking content is replaced by one neutral analysis status', () => {
    const item: IMessageThinking = {
      id: 'thinking', type: 'thinking', conversation_id: conversationId,
      position: 'left', created_at: 1,
      content: { content: 'Earlier result\nCalling the file tool', status: 'thinking' },
    };
    const { container } = render(<I18nextProvider i18n={i18n}><ProcessTraceItem item={item} /></I18nextProvider>);
    expect(container.textContent).toContain('Analyzing the request');
    expect(container.textContent).not.toContain('Earlier result');
    expect(container.textContent).not.toContain('Calling the file tool');
  });

  test('a rehydrated tool row opens its saved input and output', () => {
    const item: IMessageToolCall = {
      id: 'saved-tool', type: 'tool_call', conversation_id: conversationId,
      position: 'left', created_at: 2,
      content: {
        call_id: 'call-1', name: 'read_file', status: 'completed',
        args: { path: 'src/app.ts' }, output: 'file contents', artifacts: [],
      },
    };
    const { getByRole, getByText } = render(
      <I18nextProvider i18n={i18n}><ProcessTraceItem item={item} /></I18nextProvider>
    );
    const toggle = getByRole('button');
    expect(toggle.getAttribute('aria-expanded')).toBe('false');
    fireEvent.click(toggle);
    expect(toggle.getAttribute('aria-expanded')).toBe('true');
    expect(getByText('src/app.ts')).toBeDefined();
    expect(getByText('file contents')).toBeDefined();
  });

  test('a deferred file call remains inspectable without a red failure row', () => {
    const item: IMessageToolCall = {
      id: 'deferred-write', type: 'tool_call', conversation_id: conversationId,
      position: 'left', created_at: 2,
      content: {
        call_id: 'call-write', name: 'write_file', status: 'error',
        args: { path: 'snake_game.html' },
        output: 'Operations not executed: instruction scope changed before write',
        artifacts: [],
      },
    };
    const { container, getByRole } = render(
      <I18nextProvider i18n={i18n}><ProcessTraceItem item={item} /></I18nextProvider>
    );
    expect(container.querySelector('.turn-process-trace__row--failed')).toBeNull();
    const toggle = getByRole('button');
    expect(toggle.textContent).toContain('Did not run');
    fireEvent.click(toggle);
    expect(container.textContent).toContain('instruction scope changed before write');
  });

  test('a mixed tool group marks only its last running call as current activity', () => {
    const item: IMessageToolGroup = {
      id: 'mixed-tools', type: 'tool_group', conversation_id: conversationId,
      position: 'left', created_at: 3,
      content: [
        { call_id: 'first', name: 'first_tool', description: 'finished result', status: 'Success', render_output_as_markdown: false },
        { call_id: 'second', name: 'second_tool', description: 'working now', status: 'Executing', render_output_as_markdown: false },
      ],
    };
    const { container } = render(<I18nextProvider i18n={i18n}><ProcessTraceItem item={item} /></I18nextProvider>);
    const active = container.querySelector('.turn-process-trace__row--current-activity');
    expect(active?.textContent).toContain('second_tool');
    expect(active?.textContent).not.toContain('first_tool');
    expect(container.querySelector('.turn-process-trace__row--completed')?.textContent).toContain('first_tool');
    const settled = render(
      <I18nextProvider i18n={i18n}><ProcessTraceItem item={item} stateOverride='completed' /></I18nextProvider>
    );
    expect(settled.container.querySelectorAll('.turn-process-trace__row--completed')).toHaveLength(2);
    expect(settled.container.querySelector('.turn-process-trace__row--current-activity')).toBeNull();
  });

  test('multiple failed calls collapse into one summary until explicitly expanded', () => {
    const item: IMessageToolGroup = {
      id: 'failed-tools', type: 'tool_group', conversation_id: conversationId,
      position: 'left', created_at: 4,
      content: ['first_tool', 'second_tool', 'third_tool'].map((name, index) => ({
        call_id: `failed-${index}`,
        name,
        description: `${name} failed detail`,
        status: 'Error' as const,
        render_output_as_markdown: false,
      })),
    };
    const { container, getByRole } = render(
      <I18nextProvider i18n={i18n}><ProcessTraceItem item={item} /></I18nextProvider>
    );

    expect(container.textContent).toContain('3 operations did not complete');
    expect(container.textContent).not.toContain('first_tool');
    const toggle = getByRole('button');
    fireEvent.click(toggle);
    expect(container.textContent).toContain('first_tool');
    expect(container.textContent).toContain('third_tool');
  });

  test('identical completed operations render once with their repeat count', () => {
    const item: IMessageToolGroup = {
      id: 'repeated-tools', type: 'tool_group', conversation_id: conversationId,
      position: 'left', created_at: 5,
      content: [0, 1, 2].map((index) => ({
        call_id: `search-${index}`,
        name: 'search_code',
        description: 'searched code',
        status: 'Success' as const,
        render_output_as_markdown: false,
      })),
    };
    const { container } = render(
      <I18nextProvider i18n={i18n}><ProcessTraceItem item={item} /></I18nextProvider>
    );

    expect(container.querySelectorAll('.turn-process-trace__row')).toHaveLength(1);
    expect(container.textContent).toContain('3 times');
  });
});
