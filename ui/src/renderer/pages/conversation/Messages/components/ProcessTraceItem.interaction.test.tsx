import { cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import type { IMessageText, IMessageThinking, IMessageToolCall, IMessageToolGroup } from '@/common/chat/chatLib';
import { parseConversationId } from '@/common/types/ids';
import ProcessTraceItem from './ProcessTraceItem';
import messagesLocale from '@/renderer/services/i18n/locales/en-US/messages.json';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { messages: messagesLocale } } } });
const conversationId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000051');

afterEach(cleanup);

describe('replayed process trace', () => {
  test('shows a clean timeout as a deadline outcome and retains its failed receipt details', () => {
    const output = JSON.stringify({ state: 'timed_out', success: false, process_id: 'owned', output: { text: '' },
      cleanup: { reaped: true, errors: [], interrupt_attempted: false, terminate_attempted: true, force_kill_attempted: false } });
    const item = { id: 'timeout', type: 'tool_call', conversation_id: conversationId,
      position: 'left', created_at: 1, content: { call_id: 'timeout', name: 'poll_process', status: 'error',
        args: { process_id: 'owned', cursor: 25 }, output, artifacts: [] } } as IMessageToolCall;
    const { container, getByRole } = render(
      <I18nextProvider i18n={i18n}><ProcessTraceItem item={item} variant='receipt' /></I18nextProvider>
    );
    expect(getByRole('button').textContent).toContain('reached its time limit; process cleanup completed');
    expect(container.querySelector('.turn-process-trace__row--failed')).not.toBeNull();
    fireEvent.click(getByRole('button'));
    expect(container.textContent).toContain(output);
    expect(item.content.status).toBe('error');
  });
  test('a proven launch refusal explains that the command was not run and retains its diagnostic', () => {
    const output = JSON.stringify({ schema: 'nomifun.process-start-observation.v1', state: 'not_started',
      code: 'PROCESS_NOT_STARTED', user_code_started: false, success: false,
      message: 'The requested executable did not start.' });
    const item: IMessageToolCall = {
      id: 'launch-refusal', type: 'tool_call', conversation_id: conversationId,
      position: 'left', created_at: 1,
      content: { call_id: 'no-start', name: 'exec_command', status: 'error',
        args: { command: 'Copy-Item', args: ['-LiteralPath', '原件.txt'] }, output, artifacts: [] },
    };
    const { container, getByRole } = render(
      <I18nextProvider i18n={i18n}><ProcessTraceItem item={item} variant='receipt' /></I18nextProvider>
    );
    expect(getByRole('button').textContent).toContain('Copy-Item');
    expect(getByRole('button').textContent).toContain('did not start; command was not run');
    expect(container.querySelector('.turn-process-trace__row--failed')).not.toBeNull();
    expect(container.textContent).not.toContain('PROCESS_NOT_STARTED');
    fireEvent.click(getByRole('button'));
    expect(container.textContent).toContain(output);
  });

  test('whitespace-only assistant fragments leave no blank process block', () => {
    const item: IMessageText = {
      id: 'empty-fragment', type: 'text', conversation_id: conversationId,
      position: 'left', created_at: 1, content: { content: '\n \n\n' },
    };
    const { container } = render(<I18nextProvider i18n={i18n}><ProcessTraceItem item={item} /></I18nextProvider>);
    expect(container.childElementCount).toBe(0);
  });

  test('a model-emitted tool-call payload never becomes a public process paragraph', () => {
    const item: IMessageText = {
      id: 'raw-tool-payload', type: 'text', conversation_id: conversationId,
      position: 'left', created_at: 1,
      content: { content: '<tool_call>\n\n<function=write_file>\n<parameter=content>\n<!DOCTYPE html>' },
    };
    const { container } = render(
      <I18nextProvider i18n={i18n}><ProcessTraceItem item={item} /></I18nextProvider>
    );
    expect(container.querySelector('summary')?.textContent).toContain('Invalid tool-call format');
    expect(container.querySelector('pre')).toBeNull();
    expect(container.textContent).not.toContain('<!DOCTYPE html>');
  });

  test('intermediate assistant prose remains visible in chronological order', () => {
    const note = 'Repeating progress should remain inspectable. '.repeat(12);
    const item: IMessageText = {
      id: 'long-progress', type: 'text', conversation_id: conversationId,
      position: 'left', created_at: 1, content: { content: note },
    };
    const { container } = render(
      <MemoryRouter><I18nextProvider i18n={i18n}><ProcessTraceItem item={item} /></I18nextProvider></MemoryRouter>
    );
    const narrationText = container.querySelector('.markdown-shadow')?.shadowRoot?.textContent ?? '';
    expect(narrationText).toContain('Repeating progress should remain inspectable.');
    expect(container.textContent).not.toContain('Prepared the result');
    expect(container.querySelector('[data-testid="process-narration"]')).not.toBeNull();
    expect(container.querySelector('button')).toBeNull();
  });

  test('returned thinking has an open Markdown body that remains readable after completion', () => {
    const item: IMessageThinking = {
      id: 'thinking', type: 'thinking', conversation_id: conversationId,
      position: 'left', created_at: 1,
      content: { content: 'Earlier result\nCalling the file tool', status: 'thinking' },
    };
    const view = (completed = false) => <MemoryRouter><I18nextProvider i18n={i18n}>
      <ProcessTraceItem item={item} stateOverride={completed ? 'completed' : undefined} />
    </I18nextProvider></MemoryRouter>;
    const { container, rerender } = render(view());
    const text = () => container.querySelector('[data-thinking-process-body] .markdown-shadow')?.shadowRoot?.textContent ?? '';
    expect(text()).toContain('Earlier result');
    expect(text()).toContain('Calling the file tool');
    expect(container.querySelector('[data-thinking-process-header]')?.getAttribute('aria-expanded')).toBe('true');
    expect(container.querySelector('[data-thinking-process-header]')?.textContent).toBe('Thinking...');
    rerender(view(true));
    expect(text()).toContain('Earlier result');
    expect(container.querySelector('[data-thinking-process-header]')?.getAttribute('aria-expanded')).toBe('true');
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

  test('an expanded stage shows its file operations directly without a second group disclosure', () => {
    const item: IMessageToolGroup = {
      id: 'edited-files', type: 'tool_group', conversation_id: conversationId,
      position: 'left', created_at: 2,
      content: ['src/a.ts', 'src/b.ts', 'src/c.ts'].map((path, index) => ({
        call_id: `edit-${index}`, name: 'write_file', status: 'Success' as const,
        description: path, render_output_as_markdown: false,
      })),
    };
    const { container } = render(
      <I18nextProvider i18n={i18n}><ProcessTraceItem item={item} variant='receipt' /></I18nextProvider>
    );

    expect(container.querySelectorAll('.turn-process-trace > .turn-process-trace-tool')).toHaveLength(3);
    expect(container.textContent).toContain('Edited a.ts');
    expect(container.textContent).toContain('Edited c.ts');
  });

  test('a single stage operation keeps raw tool output closed until that operation is opened', () => {
    const item: IMessageToolCall = {
      id: 'single-write', type: 'tool_call', conversation_id: conversationId,
      position: 'left', created_at: 2,
      content: {
        call_id: 'write-one', name: 'write_file', status: 'completed',
        args: { path: 'snake_game.html' },
        output: '{"written":true}', artifacts: [],
      },
    };
    const { container, getByRole } = render(
      <I18nextProvider i18n={i18n}><ProcessTraceItem item={item} variant='receipt' /></I18nextProvider>
    );
    expect(container.textContent).toContain('Edited snake_game.html');
    expect(container.textContent).not.toContain('"written":true');
    fireEvent.click(getByRole('button'));
    expect(container.textContent).toContain('"written":true');
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
    expect(active?.closest('[data-tool-call-id]')?.getAttribute('data-tool-call-id')).toBe('second');
    expect(active?.textContent).toContain('working now');
    expect(container.querySelector('.turn-process-trace__row--completed')?.closest('[data-tool-call-id]')?.getAttribute('data-tool-call-id')).toBe('first');
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

  test('intermediate failures in a continuing turn use a recovered receipt', () => {
    const item: IMessageToolGroup = {
      id: 'recovered-tools', type: 'tool_group', conversation_id: conversationId,
      position: 'left', created_at: 4,
      content: [{
        call_id: 'failed-once', name: 'run_command', description: 'targeted test',
        status: 'Error' as const, render_output_as_markdown: false,
      }],
    };
    const { container, getByRole } = render(
      <I18nextProvider i18n={i18n}><ProcessTraceItem item={item} recoverFailures /></I18nextProvider>
    );

    expect(container.querySelector('.turn-process-trace__row--failed')).toBeNull();
    expect(container.querySelector('.turn-process-trace__row--recovered')).not.toBeNull();
    expect(container.textContent).toContain('Failed targeted test');
    fireEvent.click(getByRole('button'));
    expect(container.textContent).toContain('targeted test');
  });

  test('distinct calls with identical labels retain their own inspectable input and output', () => {
    const item = {
      id: 'repeated-tools', type: 'tool_summary' as const, created_at: 5, sourceMessageIds: [],
      messages: [0, 1, 2].map<IMessageToolCall>((index) => ({
        id: `search-${index}`, type: 'tool_call', conversation_id: conversationId, created_at: 5,
        content: {
        call_id: `search-${index}`,
        name: 'search_code',
        description: 'searched code',
        status: 'completed',
        args: { query: `query-${index}` },
        output: `result-${index}`, artifacts: [],
        },
      })),
    };
    const { container } = render(
      <I18nextProvider i18n={i18n}><ProcessTraceItem item={item} /></I18nextProvider>
    );

    const rows = container.querySelectorAll('[data-tool-call-id]');
    expect(rows).toHaveLength(3);
    expect(container.textContent).not.toContain('3 times');
    rows.forEach((row, index) => {
      fireEvent.click(row.querySelector('button')!);
      expect(row.textContent).toContain(`query-${index}`);
      expect(row.textContent).toContain(`result-${index}`);
      expect(row.querySelectorAll('button')).toHaveLength(1);
    });
  });

  test('failed calls in the journal are individually visible with a short diagnostic', () => {
    const item: IMessageToolGroup = {
      id: 'visible-failures', type: 'tool_group', conversation_id: conversationId,
      position: 'left', created_at: 5,
      content: [0, 1].map((index) => ({
        call_id: `failure-${index}`, name: 'create_draft', description: 'create_draft', status: 'Error' as const,
        render_output_as_markdown: false,
        result_display: JSON.stringify({ code: 'PLUGIN_NOT_FOUND', message: `Missing plugin-${index}` }),
      })),
    };
    const { container } = render(<I18nextProvider i18n={i18n}><ProcessTraceItem item={item} variant='receipt' /></I18nextProvider>);
    const rows = container.querySelectorAll('[data-tool-call-id]');
    expect(rows).toHaveLength(2);
    rows.forEach((row, index) => {
      expect(row.querySelector('.turn-process-trace__diagnostic')?.textContent).toContain(`Missing plugin-${index}`);
      expect(row.querySelector('.turn-process-trace-detail')).toBeNull();
      fireEvent.click(row.querySelector('button')!);
      expect(row.querySelector('.turn-process-trace-detail')?.textContent).toContain(String(item.content[index].result_display));
      expect(row.querySelectorAll('button')).toHaveLength(1);
    });
  });

  test('retry attempts retain separate call identities and outputs in journal order', () => {
    const first: IMessageToolCall = { id: 'retry-first', type: 'tool_call', conversation_id: conversationId,
      created_at: 1, content: { call_id: 'root-call', name: 'read_file', status: 'error',
        args: { path: 'first.txt' }, output: 'first failure', artifacts: [],
        retry: { retry_group_id: 'root-call', attempt_no: 1 } } };
    const next: IMessageToolCall = { ...first, id: 'retry-next', created_at: 2,
      content: { ...first.content, call_id: 'next-call', status: 'completed', output: 'second result',
        retry: { retry_group_id: 'root-call', retry_of_call_id: 'root-call', attempt_no: 2 } } };
    const { container } = render(<I18nextProvider i18n={i18n}><ProcessTraceItem
      item={{ type: 'tool_summary', id: 'retry-stage', created_at: 1, sourceMessageIds: [], messages: [first, next] }}
      variant='receipt' /></I18nextProvider>);
    const rows = container.querySelectorAll('[data-tool-call-id]');
    expect(Array.from(rows, (row) => row.getAttribute('data-tool-call-id'))).toEqual(['root-call', 'next-call']);
    expect(rows[1].textContent).toContain('Attempt 2');
    rows.forEach((row) => fireEvent.click(row.querySelector('button')!));
    expect(rows[0].textContent).toContain('first failure');
    expect(rows[1].textContent).toContain('second result');
  });
});
