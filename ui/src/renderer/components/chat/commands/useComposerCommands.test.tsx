import '../../../../../test/setup-dom.ts';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { Message } from '@arco-design/web-react';
import { useRef, useState } from 'react';
import { useCompositionInput } from '@/renderer/hooks/chat/useCompositionInput';
import { useComposerCommands, type ComposerCommand } from './useComposerCommands';
import { commandQuery, resolveComposerCommand, validateComposerCommands } from './types';

afterEach(() => { cleanup(); mock.restore(); });

function mount(commands: ComposerCommand[], initial = '/clear') {
  const sent = mock();
  let changeTarget!: (target: string) => void;
  function Harness() {
    const [input, setInput] = useState(initial);
    const [target, setTarget] = useState('first'); changeTarget = setTarget;
    const anchor = useRef<HTMLDivElement>(null);
    const controller = useComposerCommands({ targetKey: target, input, setInput, commands, anchorRef: anchor });
    const { compositionHandlers, createKeyDownHandler } = useCompositionInput();
    const submit = () => { if (!controller.dispatchInput()) sent(input); };
    return <div ref={anchor} style={{ overflow: 'hidden' }}>
      <textarea aria-label='Draft' value={input} onChange={event => setInput(event.target.value)} {...compositionHandlers}
        onKeyDown={createKeyDownHandler(submit, controller.onKeyDown)} />
      <button onClick={submit}>Submit</button>{controller.menu}<output>{controller.busy ? 'busy' : 'idle'}</output>
    </div>;
  }
  const view = render(<Harness />);
  return { ...view, sent, change: (value: string) => fireEvent.change(view.getByRole('textbox'), { target: { value } }),
    target: (value: string) => act(() => changeTarget(value)) };
}

test.each(['button', 'enter', 'menu'])('known command uses the same action for %s and never sends a message', async gesture => {
  const execute = mock();
  const view = mount([{ id: 'session.clear-context', name: 'clear', description: 'Clear context', execute }]);
  if (gesture === 'button') fireEvent.click(view.getByRole('button', { name: 'Submit' }));
  else if (gesture === 'enter') fireEvent.keyDown(view.getByRole('textbox'), { key: 'Enter' });
  else fireEvent.click(await view.findByRole('option'));
  await waitFor(() => expect(execute).toHaveBeenCalledTimes(1));
  expect(view.sent).not.toHaveBeenCalled();
  await waitFor(() => expect((view.getByRole('textbox') as HTMLTextAreaElement).value).toBe(''));
});

test('commands are portaled outside clipping panes', async () => {
  const view = mount([{ id: 'session.clear-context', name: 'clear', description: '', execute: mock() }]);
  await view.findByRole('option');
  const panel = document.querySelector('[data-composer-command-overlay]')!;
  expect(panel).not.toBeNull();
  expect(view.container.contains(panel)).toBe(false);
  expect(document.body.contains(panel)).toBe(true);
});

test('disabled and malformed commands are consumed without effects or ordinary submission', async () => {
  const warning = spyOn(Message, 'warning').mockImplementation(() => () => {});
  const execute = mock();
  const view = mount([{ id: 'session.clear-context', name: 'clear', description: '', disabledReason: 'Wait for the current task' },
    { id: 'files.open', name: 'open', description: '', execute }]);
  expect((await view.findByRole('option')).getAttribute('aria-disabled')).toBe('true');
  fireEvent.click(view.getByRole('button', { name: 'Submit' }));
  view.change('/open unexpected arguments');
  fireEvent.click(view.getByRole('button', { name: 'Submit' }));
  await act(async () => {});
  expect(warning).toHaveBeenCalledTimes(2);
  expect(execute).not.toHaveBeenCalled(); expect(view.sent).not.toHaveBeenCalled();
});

test('a failed action retains its command draft and a pending action is single flight', async () => {
  let reject!: (error: Error) => void;
  const error = spyOn(Message, 'error').mockImplementation(() => () => {});
  const execute = mock(() => new Promise<void>((_, fail) => { reject = fail; }));
  const view = mount([{ id: 'session.clear-context', name: 'clear', description: '', execute }]);
  fireEvent.click(view.getByRole('button', { name: 'Submit' }));
  fireEvent.keyDown(view.getByRole('textbox'), { key: 'Enter' });
  await waitFor(() => expect(execute).toHaveBeenCalledTimes(1));
  await act(async () => reject(new Error('offline')));
  expect((view.getByRole('textbox') as HTMLTextAreaElement).value).toBe('/clear');
  expect(error).toHaveBeenCalledTimes(1); expect(view.sent).not.toHaveBeenCalled();
});

test('an unchanged failed action reuses its request key; an explicit new draft starts a new action', async () => {
  spyOn(Message, 'error').mockImplementation(() => () => {});
  const keys: string[] = [];
  const execute = mock(async ({ idempotencyKey }: { idempotencyKey: string }) => {
    keys.push(idempotencyKey);
    if (keys.length === 1) throw new Error('response was lost');
  });
  const view = mount([{ id: 'session.clear-context', name: 'clear', description: '', execute }]);
  fireEvent.click(view.getByRole('button', { name: 'Submit' }));
  await waitFor(() => expect(view.getByRole('status').textContent).toBe('idle'));
  fireEvent.click(view.getByRole('button', { name: 'Submit' }));
  await waitFor(() => expect((view.getByRole('textbox') as HTMLTextAreaElement).value).toBe(''));
  expect(keys).toHaveLength(2); expect(keys[0]).toBe(keys[1]);
  view.change('/clear'); fireEvent.click(view.getByRole('button', { name: 'Submit' }));
  await waitFor(() => expect(keys).toHaveLength(3));
  expect(keys[2]).not.toBe(keys[1]);
});

test.each(['edit', 'switch', 'unmount'])('late command completion preserves a newer draft or owner (%s)', async change => {
  let finish!: () => void;
  const execute = mock(() => new Promise<void>(resolve => { finish = resolve; }));
  const view = mount([{ id: 'session.clear-context', name: 'clear', description: '', execute }]);
  fireEvent.click(view.getByRole('button', { name: 'Submit' }));
  await waitFor(() => expect(execute).toHaveBeenCalledTimes(1));
  if (change === 'switch') view.target('second');
  view.change('new draft');
  if (change === 'unmount') view.unmount();
  await act(async () => finish());
  if (change !== 'unmount') expect((view.getByRole('textbox') as HTMLTextAreaElement).value).toBe('new draft');
  expect(view.sent).not.toHaveBeenCalled();
});

test('IME confirmation and system editing keys do not activate commands; unknown slash text remains ordinary input', async () => {
  const execute = mock();
  const view = mount([{ id: 'session.clear-context', name: 'clear', description: '', execute }]);
  const input = view.getByRole('textbox');
  fireEvent.compositionStart(input); fireEvent.keyDown(input, { key: 'Enter' });
  fireEvent.compositionEnd(input); fireEvent.keyDown(input, { key: 'Enter', keyCode: 229 });
  fireEvent.keyDown(input, { key: 'c', ctrlKey: true });
  await act(async () => {});
  expect(execute).not.toHaveBeenCalled();
  view.change('/usr/bin/env'); fireEvent.click(view.getByRole('button', { name: 'Submit' }));
  expect(view.sent).toHaveBeenCalledWith('/usr/bin/env');
});

test('catalog changes clamp navigation and preserve escape dismissal', async () => {
  const first = mock(); const second = mock();
  const commands: ComposerCommand[] = [
    { id: 'files.open', name: 'open', description: '', execute: first },
    { id: 'session.clear-context', name: 'clear', description: '', execute: second },
  ];
  const view = mount(commands, '/');
  fireEvent.keyDown(view.getByRole('textbox'), { key: 'ArrowDown' });
  fireEvent.keyDown(view.getByRole('textbox'), { key: 'Escape' });
  expect(view.queryByRole('option')).toBeNull();
  view.change('/clear '); fireEvent.click(view.getByRole('button', { name: 'Submit' }));
  await waitFor(() => expect(second).toHaveBeenCalledTimes(1));
  expect(first).not.toHaveBeenCalled(); expect(view.sent).not.toHaveBeenCalled();
});

test('parser reserves only complete known command names and rejects ambiguous catalogs', () => {
  const commands: ComposerCommand[] = [{ id: 'files.open', name: 'open', description: '', execute: mock() }];
  expect(commandQuery('/')).toBe(''); expect(commandQuery('/open\n')).toBeNull();
  expect(resolveComposerCommand('Discuss /open', commands)).toBeNull();
  expect(resolveComposerCommand('/open/path', commands)).toBeNull();
  expect(resolveComposerCommand('/open x', commands)?.hasArguments).toBe(true);
  expect(resolveComposerCommand('/OPEN ', commands)?.command.id).toBe('files.open');
  expect(() => validateComposerCommands([...commands, { id: 'other', name: 'OPEN', description: '', execute: mock() }])).toThrow();
});
