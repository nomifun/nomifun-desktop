import { afterEach, expect, mock, test } from 'bun:test';
import { cleanup, fireEvent, render } from '@testing-library/react';
import { createInstance } from 'i18next';
import common from '@/renderer/services/i18n/locales/en-US/common.json';
import conversation from '@/renderer/services/i18n/locales/en-US/conversation.json';
import { WorkspaceReadFailure } from './WorkspaceReadFailure';

afterEach(cleanup);
const translations = createInstance();
await translations.init({ lng: 'en-US', resources: { 'en-US': { translation: { common, conversation } } } });

test('a first-read failure exposes a retry action and does not describe an empty workspace', () => {
  const retry = mock(() => {});
  const view = render(<WorkspaceReadFailure t={translations.t} hasSnapshot={false} retrying={false} onRetry={retry} />);
  expect(view.getByRole('alert').textContent).toContain(conversation.workspace.readErrorTitle);
  expect(view.queryByText(conversation.workspace.empty)).toBeNull();
  expect(view.queryByText(conversation.workspace.staleFiles)).toBeNull();
  fireEvent.click(view.getByRole('button', { name: common.retry }));
  expect(retry).toHaveBeenCalledTimes(1);
  view.rerender(<WorkspaceReadFailure t={translations.t} hasSnapshot={false} retrying={true} onRetry={retry} />);
  expect(view.getByRole('button', { name: common.retry }).hasAttribute('disabled')).toBe(true);
});

test('a retained file snapshot has an explicit stale-content warning', () => {
  const view = render(<WorkspaceReadFailure t={translations.t} hasSnapshot retrying={false} onRetry={() => {}} />);
  expect(view.getByRole('alert').textContent).toContain(conversation.workspace.staleFiles);
});
