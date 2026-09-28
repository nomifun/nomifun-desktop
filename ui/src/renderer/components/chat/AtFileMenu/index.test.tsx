import { afterEach, expect, mock, test } from 'bun:test';
import { cleanup, fireEvent, render } from '@testing-library/react';
import AtFileMenu from './index';

afterEach(cleanup);

const item = { path: '/workspace/known.txt', name: 'known.txt', relativePath: 'known.txt', isFile: true };
const props = {
  activeIndex: 0,
  emptyText: 'No files found',
  items: [] as typeof item[],
  label: 'File mentions',
  loading: false,
  loadingText: 'Loading files',
  onHoverItem: () => {},
  onSelectItem: () => {},
};

test('a failed file listing exposes an error and retry instead of a no-match result', () => {
  const failure = { message: 'Cannot read files', retryLabel: 'Retry', onRetry: mock(() => {}) };
  const view = render(<AtFileMenu {...{ ...props, failure }} />);
  expect(view.getByRole('alert').textContent).toContain(failure.message);
  expect(view.getByRole('button', { name: 'Retry' })).toBeTruthy();
  expect(view.queryByText('No files found')).toBeNull();
});

test('loading a new inventory does not leave old candidates selectable', () => {
  const view = render(<AtFileMenu {...props} items={[item]} loading />);
  expect(view.queryAllByRole('option')).toHaveLength(0);
  expect(view.getByText('Loading files')).toBeTruthy();
});

test('retry preserves composer focus and does not expose stale options or submit the form', () => {
  const retry = mock(() => {});
  const submit = mock(() => {});
  const failure = { message: 'Cannot read files', retryLabel: 'Retry', onRetry: retry };
  const view = render(<form onSubmit={submit}><textarea aria-label='Draft' defaultValue='@known' /><AtFileMenu {...props} items={[item]} failure={failure} /></form>);
  const draft = view.getByRole('textbox', { name: 'Draft' });
  draft.focus();
  const button = view.getByRole('button', { name: 'Retry' });
  expect(fireEvent.mouseDown(button)).toBe(false);
  fireEvent.click(button);
  expect(document.activeElement).toBe(draft);
  expect(retry).toHaveBeenCalledTimes(1);
  expect(submit).not.toHaveBeenCalled();
  expect(view.queryAllByRole('option')).toHaveLength(0);
});

test('a ready candidate can still be selected with its original path', () => {
  const select = mock(() => {});
  const view = render(<AtFileMenu {...props} items={[item]} onSelectItem={select} />);
  fireEvent.mouseDown(view.getByRole('option'));
  expect(select).toHaveBeenCalledWith(item);
});
