import '../../../../../../test/setup-dom.ts';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, expect, test } from 'bun:test';
import { useState } from 'react';
import { withCanvasTestI18n } from '../components/canvasI18nTestUtils';
import CreativeCanvasTitle from './CreativeCanvasTitle';

afterEach(cleanup);

function mount(onRename: (title: string) => Promise<void> = async () => {}, disabled = false) {
  const calls: string[] = [];
  function Harness() {
    const [title, setTitle] = useState('My canvas');
    return <CreativeCanvasTitle title={title} disabled={disabled} onRename={async next => {
      calls.push(next);
      await onRename(next);
      setTitle(next);
    }} />;
  }
  return { ...render(withCanvasTestI18n(<Harness />)), calls };
}

test('double-click selects the title and Enter saves a trimmed name once', async () => {
  let finish!: () => void;
  const view = mount(() => new Promise<void>(resolve => { finish = resolve; }));
  fireEvent.doubleClick(view.getByRole('heading'));
  const input = view.getByRole('textbox') as HTMLInputElement;
  expect(document.activeElement).toBe(input);
  expect(input.selectionEnd).toBe('My canvas'.length);
  fireEvent.change(input, { target: { value: '  Renamed canvas  ' } });
  fireEvent.keyDown(input, { key: 'Enter' });
  fireEvent.blur(input);
  expect(view.calls).toEqual(['Renamed canvas']);
  expect(input.readOnly).toBe(true);
  await act(async () => finish());
  expect(view.getByRole('heading').textContent).toBe('Renamed canvas');
});

test('blur saves and Escape, blank or unchanged names do not send requests', async () => {
  const view = mount();
  for (const name of ['My canvas', '   ', 'Cancelled']) {
    fireEvent.doubleClick(view.getByRole('heading'));
    const input = view.getByRole('textbox');
    fireEvent.change(input, { target: { value: name } });
    if (name === 'Cancelled') fireEvent.keyDown(input, { key: 'Escape' });
    fireEvent.blur(input);
    expect(view.queryByRole('textbox')).toBeNull();
  }
  expect(view.calls).toEqual([]);
  fireEvent.keyDown(view.getByRole('heading'), { key: 'F2' });
  fireEvent.change(view.getByRole('textbox'), { target: { value: 'Saved on blur' } });
  fireEvent.blur(view.getByRole('textbox'));
  await waitFor(() => expect(view.getByRole('heading').textContent).toBe('Saved on blur'));
});

test('failed rename retains the draft for retry and IME Enter does not submit', async () => {
  let fail = true;
  const view = mount(async () => { if (fail) throw new Error('offline'); });
  fireEvent.doubleClick(view.getByRole('heading'));
  const input = view.getByRole('textbox');
  fireEvent.change(input, { target: { value: 'Retry title' } });
  fireEvent.keyDown(input, { key: 'Enter', isComposing: true });
  expect(view.calls).toEqual([]);
  fireEvent.keyDown(input, { key: 'Enter' });
  await waitFor(() => expect(view.getByRole('alert')).toBeTruthy());
  expect((view.getByRole('textbox') as HTMLInputElement).value).toBe('Retry title');
  fail = false;
  fireEvent.keyDown(input, { key: 'Enter' });
  await waitFor(() => expect(view.getByRole('heading').textContent).toBe('Retry title'));
});

test('disabled titles cannot enter edit mode', () => {
  const view = mount(undefined, true);
  fireEvent.doubleClick(view.getByRole('heading'));
  fireEvent.keyDown(view.getByRole('heading'), { key: 'Enter' });
  expect(view.queryByRole('textbox')).toBeNull();
});
