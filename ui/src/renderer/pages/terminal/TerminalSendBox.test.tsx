import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import type { ComponentProps } from 'react';
import { act, cleanup, render } from '@testing-library/react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { Message } from '@arco-design/web-react';
import * as composer from '@/renderer/components/chat/SendBox';
import TerminalSendBox from './TerminalSendBox';
import type { XtermViewHandle } from './XtermView';

const i18n = createInstance();
await i18n.init({ lng: 'en', resources: { en: { translation: {} } } });
const restore: Array<() => void> = [];
afterEach(() => { cleanup(); restore.splice(0).reverse().forEach(fn => fn()); });
function fixture() {
  let props!: ComponentProps<typeof composer.default>;
  const child = spyOn(composer, 'default').mockImplementation(value => { props = value; return null; });
  const error = spyOn(Message, 'error').mockImplementation(() => () => {});
  restore.push(() => child.mockRestore(), () => error.mockRestore());
  const writes: Array<{ text: string; resolve: () => void; reject: (error: unknown) => void }> = [];
  const api: XtermViewHandle = {
    writeToPty: text => new Promise<void>((resolve, reject) => writes.push({ text, resolve, reject })),
    isBracketedPaste: () => false, clear: mock(), reset: mock(), focus: mock(),
  };
  const terminalApi = { current: api as XtermViewHandle | null };
  const clear = mock();
  const view = render(<I18nextProvider i18n={i18n}><TerminalSendBox terminalApi={terminalApi} onClearView={clear} /></I18nextProvider>);
  return {
    view, terminalApi, api, writes, error, clear,
    props: () => props,
    change: (text: string) => act(() => props.onChange?.(text)),
    // Shared SendBox composeAndClear calls onChange('') before onSend. Exercise
    // this public boundary while retaining all terminal-specific production logic.
    send: (text: string) => {
      let result!: Promise<void>;
      act(() => { props.onChange?.(''); result = props.onSend(text).catch(() => {}); });
      return result;
    },
  };
}

test.each(['new draft', ''])('failed send preserves a later edited draft (%s)', async next => {
  const f = fixture();
  const pending = f.send('old command');
  f.change('edited'); f.change(next);
  await act(async () => { f.writes[0]!.reject('offline'); await pending; });
  expect(f.props().value).toBe(next);
  expect(f.error).toHaveBeenCalledTimes(1);
});

test('failed send restores the original text when the draft is untouched', async () => {
  const f = fixture();
  const pending = f.send('old command  ');
  await act(async () => { f.writes[0]!.reject('offline'); await pending; });
  expect(f.props().value).toBe('old command  ');
  expect(f.error).toHaveBeenCalledTimes(1);
});

test('late send and interrupt failures after unmount are silent', async () => {
  const f = fixture();
  const pending = f.send('command');
  act(() => f.props().onSlashBuiltinCommand?.('interrupt'));
  f.view.unmount();
  await act(async () => { f.writes.forEach(write => write.reject('late')); await pending; });
  expect(f.error).not.toHaveBeenCalled();
});

test('an old terminal handle failure cannot restore text or notify after replacement', async () => {
  const f = fixture();
  const pending = f.send('command');
  f.terminalApi.current = { ...f.api };
  await act(async () => { f.writes[0]!.reject('late'); await pending; });
  expect(f.props().value).toBe('');
  expect(f.error).not.toHaveBeenCalled();
});

test('clear uses the shared composer clear-context entry and clears only the view/draft', () => {
  const f = fixture();
  f.change('/clear');
  act(() => { void f.props().onClearContext?.(); });
  expect(f.clear).toHaveBeenCalledTimes(1);
  expect(f.props().value).toBe('');
  expect(f.writes).toEqual([]);
});

test('plain and bracketed multiline submissions retain their byte contracts', async () => {
  const f = fixture();
  const plain = f.send('a\nb  ');
  expect(f.writes[0]!.text).toBe('a\rb\r');
  await act(async () => { f.writes[0]!.resolve(); await plain; });
  f.api.isBracketedPaste = () => true;
  const bracketed = f.send('a\r\nb');
  expect(f.writes[1]!.text).toBe('\x1b[200~a\rb\x1b[201~\r');
  await act(async () => { f.writes[1]!.resolve(); await bracketed; });
  await act(async () => { await f.send('  '); });
  expect(f.writes.length).toBe(2);
  expect(f.error).not.toHaveBeenCalled();
});
